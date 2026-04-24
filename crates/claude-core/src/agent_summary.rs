//! Periodic background summarisation for sub-agents (coordinator mode).
//!
//! Port of `src/services/AgentSummary/agentSummary.ts`. The TS version
//! forks the sub-agent's conversation every ~30s via `runForkedAgent()`
//! to generate a 1-2 sentence progress summary for UI display.
//!
//! Rust scope: we port the prompt builder + the scheduler loop shape.
//! The actual forked-agent machinery (cacheSafeParams,
//! getAgentTranscript, updateAgentSummary, filterIncompleteToolCalls)
//! is all tied to TS-specific runtime state (bootstrap/state, Task.ts,
//! sessionStorage) that Rust hasn't ported in this form. Callers that
//! want full TS parity will need to implement the forked-agent layer
//! first; this module gives them the scheduling + prompt scaffolding.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Result;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Summary is requested every 30 seconds. Matches TS.
pub const SUMMARY_INTERVAL_MS: u64 = 30_000;

/// Build the user-prompt text that asks the forked sub-agent for a
/// present-tense 3-5 word progress summary. Includes the previous
/// summary so successive calls don't repeat themselves. Mirrors TS
/// `buildSummaryPrompt` — text verbatim so prompt-caching prefix-
/// matches when both implementations hit the API.
pub fn build_summary_prompt(previous_summary: Option<&str>) -> String {
    let prev_line = match previous_summary {
        Some(p) => format!("\nPrevious: \"{}\" — say something NEW.\n", p),
        None => String::new(),
    };

    format!(
        "Describe your most recent action in 3-5 words using present tense (-ing). Name the file or function, not the branch. Do not use tools.\n{prev}\nGood: \"Reading runAgent.ts\"\nGood: \"Fixing null check in validate.ts\"\nGood: \"Running auth module tests\"\nGood: \"Adding retry logic to fetchUser\"\n\nBad (past tense): \"Analyzed the branch diff\"\nBad (too vague): \"Investigating the issue\"\nBad (too long): \"Reviewing full branch diff and AgentTool.tsx integration\"\nBad (branch name): \"Analyzed adam/background-summary branch diff\"",
        prev = prev_line
    )
}

/// A callback that produces a summary given the previous one. The
/// wiring-in stage (fork + run + extract first text block) is the
/// caller's responsibility; typically this is a closure that calls
/// runForkedAgent (or its Rust equivalent, once wired) and extracts
/// the first assistant text.
pub type SummaryFn = Arc<
    dyn Fn(
            Option<String>,
            CancellationToken,
        ) -> futures_util::future::BoxFuture<'static, Result<Option<String>>>
        + Send
        + Sync,
>;

/// Running summariser handle. Drop or call `stop()` to cancel.
pub struct SummarizationHandle {
    stopped: Arc<AtomicBool>,
    task: Option<JoinHandle<()>>,
    cancel: CancellationToken,
}

impl SummarizationHandle {
    pub fn stop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.cancel.cancel();
        if let Some(h) = self.task.take() {
            h.abort();
        }
    }
}

impl Drop for SummarizationHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Start a background summarisation loop that calls `summarize(prev)`
/// every ~SUMMARY_INTERVAL_MS. Each successful summary is passed to
/// `on_summary`. The timer is RESET on completion (not initiation) so
/// overlapping summaries can't stack up — matches TS behaviour.
pub fn start_agent_summarization<F>(summarize: SummaryFn, on_summary: F) -> SummarizationHandle
where
    F: Fn(String) + Send + Sync + 'static,
{
    let stopped = Arc::new(AtomicBool::new(false));
    let stopped_task = stopped.clone();
    let cancel = CancellationToken::new();
    let cancel_task = cancel.clone();

    let task = tokio::spawn(async move {
        let mut previous: Option<String> = None;
        loop {
            if stopped_task.load(Ordering::SeqCst) || cancel_task.is_cancelled() {
                break;
            }
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_millis(SUMMARY_INTERVAL_MS)) => {}
                _ = cancel_task.cancelled() => break,
            }
            if stopped_task.load(Ordering::SeqCst) {
                break;
            }
            match summarize(previous.clone(), cancel_task.clone()).await {
                Ok(Some(text)) => {
                    let trimmed = text.trim().to_string();
                    if !trimmed.is_empty() {
                        previous = Some(trimmed.clone());
                        on_summary(trimmed);
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::debug!("[AgentSummary] summarize returned error: {}", e);
                }
            }
        }
    });

    SummarizationHandle {
        stopped,
        task: Some(task),
        cancel,
    }
}

/// Extract the first trimmed text block from a vec of assistant-message
/// texts. Callers that drive a summary manually (without the scheduler)
/// use this to convert a raw response into a single-line summary.
pub fn first_nonempty_text(blocks: &[String]) -> Option<String> {
    for b in blocks {
        let t = b.trim();
        if !t.is_empty() {
            return Some(t.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_prompt_includes_examples() {
        let p = build_summary_prompt(None);
        assert!(p.contains("Good: \"Reading runAgent.ts\""));
        assert!(p.contains("Bad (past tense)"));
        assert!(!p.contains("Previous: "));
    }

    #[test]
    fn build_prompt_mentions_previous() {
        let p = build_summary_prompt(Some("Reading x.ts"));
        assert!(p.contains("Previous: \"Reading x.ts\""));
        assert!(p.contains("say something NEW"));
    }

    #[test]
    fn first_nonempty_text_skips_empties() {
        let v = vec!["".into(), "  ".into(), "hello".into(), "other".into()];
        assert_eq!(first_nonempty_text(&v).as_deref(), Some("hello"));
    }

    #[test]
    fn first_nonempty_text_returns_none_when_all_empty() {
        let v = vec!["".into(), "  ".into()];
        assert!(first_nonempty_text(&v).is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn scheduler_calls_summarize_after_interval() {
        use std::sync::Mutex;
        let calls = Arc::new(Mutex::new(0u32));
        let calls2 = calls.clone();

        let summarize: SummaryFn = Arc::new(move |_prev, _cancel| {
            let calls = calls2.clone();
            Box::pin(async move {
                *calls.lock().unwrap() += 1;
                Ok(Some(format!("iter {}", *calls.lock().unwrap())))
            })
        });

        let collected = Arc::new(Mutex::new(Vec::<String>::new()));
        let collected2 = collected.clone();
        let mut handle = start_agent_summarization(summarize, move |s| {
            collected2.lock().unwrap().push(s);
        });

        tokio::time::advance(std::time::Duration::from_millis(SUMMARY_INTERVAL_MS + 10)).await;
        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_millis(SUMMARY_INTERVAL_MS + 10)).await;
        tokio::task::yield_now().await;
        handle.stop();

        let c = *calls.lock().unwrap();
        assert!(c >= 1, "expected at least one summariser call, got {c}");
    }
}
