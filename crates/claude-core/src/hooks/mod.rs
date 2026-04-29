pub mod aggregation;
pub mod compact_hooks;
pub mod matching;
pub mod runner;
pub mod ssrf;
pub mod tool_hooks;
pub mod types;

// ── Process-wide HookRunner handle ───────────────────────────────────────────
//
// Tools that run outside the REPL loop (e.g. TaskCreateTool firing
// TaskCreated hooks from claude-tools) need a way to reach the configured
// HookRunner without plumbing it through every ToolUseContext construction
// site. The CLI/TUI entry point installs one at startup via
// `set_global_runner`; tools that want to fire an event call
// `get_global_runner` and no-op if nothing was installed (matches the TS
// behaviour where hooks are optional config).

use std::sync::{Arc, OnceLock, RwLock};

static GLOBAL_RUNNER: OnceLock<RwLock<Option<Arc<runner::HookRunner>>>> = OnceLock::new();

/// Install or replace the process-wide HookRunner.
///
/// TS refreshes hook config when settings change; replacement keeps the Rust
/// global handle compatible with that runtime flow.
pub fn set_global_runner(runner: Arc<runner::HookRunner>) {
    let lock = GLOBAL_RUNNER.get_or_init(|| RwLock::new(None));
    if let Ok(mut guard) = lock.write() {
        *guard = Some(runner);
    }
}

/// Fetch the registered HookRunner if one has been installed.
pub fn get_global_runner() -> Option<Arc<runner::HookRunner>> {
    GLOBAL_RUNNER
        .get()
        .and_then(|lock| lock.read().ok().and_then(|guard| guard.clone()))
}

/// Update the process-wide HookRunner cwd while preserving its loaded config.
pub fn set_global_runner_cwd(cwd: String) {
    let lock = GLOBAL_RUNNER.get_or_init(|| RwLock::new(None));
    if let Ok(mut guard) = lock.write() {
        if let Some(runner) = guard.as_ref() {
            *guard = Some(Arc::new(runner.with_cwd(cwd)));
        }
    }
}

/// Fire `CwdChanged` hooks after the session working directory changes.
///
/// TS updates its cwd watcher and executes these hooks from the new cwd. Rust
/// mirrors that by cloning the global runner with `new_cwd`, running the hook,
/// and then storing that cwd for future hook execution.
pub async fn fire_cwd_changed(old_cwd: &str, new_cwd: &str) -> Vec<String> {
    if old_cwd == new_cwd {
        return Vec::new();
    }
    let Some(runner) = get_global_runner() else {
        return Vec::new();
    };
    let runner = runner.with_cwd(new_cwd.to_string());
    let extra = serde_json::json!({
        "old_cwd": old_cwd,
        "new_cwd": new_cwd,
    });
    let results = runner
        .run_hooks(&types::HookEvent::CwdChanged, extra, None, None, None, None)
        .await;
    set_global_runner_cwd(new_cwd.to_string());
    results
        .individual_results
        .iter()
        .filter(|result| result.outcome != types::HookOutcome::Success)
        .filter_map(|result| {
            if !result.stdout.is_empty() {
                Some(result.stdout.clone())
            } else if !result.stderr.is_empty() {
                Some(result.stderr.clone())
            } else {
                None
            }
        })
        .collect()
}

/// Fire a `StopFailure` hook via the global runner, if one is installed.
/// Logs blocking errors via tracing and returns the joined error message.
/// Returns `None` if no runner is installed, no hooks matched, or no hook
/// produced a blocking error.
///
/// Mirrors TS `executeStopFailureHooks` entry behaviour (without the full
/// lastMessage plumbing — callers pass the error string directly).
pub async fn fire_stop_failure(reason: &str) -> Option<String> {
    let runner = get_global_runner()?;
    let extra = serde_json::json!({ "error": reason });
    let result = runner
        .run_hooks(
            &types::HookEvent::StopFailure,
            extra,
            None,
            None,
            None,
            None,
        )
        .await;
    if result.blocking_errors.is_empty() {
        return None;
    }
    let msg = result
        .blocking_errors
        .iter()
        .map(runner::get_stop_hook_message)
        .collect::<Vec<_>>()
        .join("\n");
    tracing::warn!("StopFailure hook feedback: {}", msg);
    Some(msg)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StopHookDecision {
    pub blocking_messages: Vec<String>,
    pub prevent_continuation: bool,
    pub stop_reason: Option<String>,
}

/// Fire `Stop` hooks at the end of a normal assistant turn.
///
/// Mirrors the TS `handleStopHooks` query integration for the main-thread
/// Stop event: blocking errors are formatted as model-visible user feedback,
/// and preventContinuation stops the loop without adding another request.
pub async fn fire_stop(
    last_assistant_message: Option<&str>,
    stop_hook_active: bool,
) -> StopHookDecision {
    let Some(runner) = get_global_runner() else {
        return StopHookDecision::default();
    };
    let extra = serde_json::json!({
        "stop_hook_active": stop_hook_active,
        "last_assistant_message": last_assistant_message,
    });
    let result = runner
        .run_hooks(&types::HookEvent::Stop, extra, None, None, None, None)
        .await;
    StopHookDecision {
        blocking_messages: result
            .blocking_errors
            .iter()
            .map(runner::get_stop_hook_message)
            .collect(),
        prevent_continuation: result.prevent_continuation,
        stop_reason: result.stop_reason,
    }
}

// Re-export the most commonly used types at the module level.
pub use aggregation::aggregate_hook_results;
pub use compact_hooks::{
    run_post_compact_hooks, run_pre_compact_hooks, PostCompactHookOutput, PreCompactHookOutput,
};
pub use matching::{get_matching_hooks, matches_pattern, resolve_match_query, MatchedHook};
pub use runner::{
    get_pre_tool_hook_blocking_message, get_stop_hook_message, get_task_completed_hook_message,
    get_task_created_hook_message, get_teammate_idle_hook_message,
    get_user_prompt_submit_hook_blocking_message, HookRunner,
};
pub use tool_hooks::{
    resolve_hook_permission_decision, run_post_tool_use_failure_hooks, run_post_tool_use_hooks,
    run_pre_tool_use_hooks, PostToolUseFailureHookDecision, PostToolUseHookDecision,
    PreToolUseHookDecision, ResolvedPermission, RuleCheckResult,
};
pub use types::*;
