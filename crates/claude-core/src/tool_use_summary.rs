//! Tool-use summary generator.
//!
//! Port of `src/services/toolUseSummary/toolUseSummaryGenerator.ts`.
//! Calls the secondary (Haiku) model with a batch of completed tools +
//! their inputs/outputs and asks for a one-line summary used in mobile
//! app progress rows.

use anyhow::Result;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::constants::display::E_TOOL_USE_SUMMARY_GENERATION_FAILED;
use crate::secondary_model;

/// System-prompt text used by the summary request. Verbatim from TS
/// `TOOL_USE_SUMMARY_SYSTEM_PROMPT` so prompt-caching can prefix-match.
pub const TOOL_USE_SUMMARY_SYSTEM_PROMPT: &str = "Write a short summary label describing what these tool calls accomplished. It appears as a single-line row in a mobile app and truncates around 30 characters, so think git-commit-subject, not sentence.

Keep the verb in past tense and the most distinctive noun. Drop articles, connectors, and long location context first.

Examples:
- Searched in auth/
- Fixed NPE in UserService
- Created signup endpoint
- Read config.json
- Ran failing tests";

/// One tool call in a batch. `input` / `output` are JSON values, not
/// necessarily strings — we stringify and truncate before prompting.
#[derive(Debug, Clone)]
pub struct ToolInfo {
    pub name: String,
    pub input: Value,
    pub output: Value,
}

/// Generate a <=30-character summary of a tool batch via the secondary
/// model. Returns `Ok(None)` when:
///   - the input batch is empty
///   - no secondary model is registered
///   - the model returns empty content
///
/// Returns `Err` for cancellation / model-call errors; callers that
/// treat summaries as best-effort should log the error via the
/// `E_TOOL_USE_SUMMARY_GENERATION_FAILED` error ID.
pub async fn generate_tool_use_summary(
    tools: &[ToolInfo],
    last_assistant_text: Option<&str>,
    cancel: CancellationToken,
) -> Result<Option<String>> {
    if tools.is_empty() {
        return Ok(None);
    }
    let Some(model) = secondary_model::get_global() else {
        return Ok(None);
    };

    let tool_summaries = tools
        .iter()
        .map(|t| {
            format!(
                "Tool: {}\nInput: {}\nOutput: {}",
                t.name,
                truncate_json(&t.input, 300),
                truncate_json(&t.output, 300),
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    let context_prefix = match last_assistant_text {
        Some(txt) if !txt.is_empty() => {
            let mut cut = 200.min(txt.len());
            while !txt.is_char_boundary(cut) {
                cut -= 1;
            }
            format!(
                "User's intent (from assistant's last message): {}\n\n",
                &txt[..cut]
            )
        }
        _ => String::new(),
    };

    let user_prompt = format!(
        "{}{}Tools completed:\n\n{}\n\nLabel:",
        context_prefix,
        // Keeping an empty middle segment matches TS so prompt-caching
        // lines up at the boundary if both implementations hit the API.
        "",
        tool_summaries,
    );

    // Our SecondaryModel trait is summarize(user_prompt, cancel) — it does
    // not accept an explicit system prompt separately. Inlining the system
    // text + user prompt yields the same behaviour for the Haiku-backed
    // implementation (single-shot prompt) while keeping the signature
    // stable. If callers later need distinct system+user slots we can
    // extend the trait.
    let full_prompt = format!(
        "{system}\n\n{user}",
        system = TOOL_USE_SUMMARY_SYSTEM_PROMPT,
        user = user_prompt,
    );

    let summary = model
        .summarize(&full_prompt, cancel)
        .await
        .map_err(|e| anyhow::anyhow!("errorId={}: {}", E_TOOL_USE_SUMMARY_GENERATION_FAILED, e))?;
    let summary = summary.trim();
    if summary.is_empty() {
        Ok(None)
    } else {
        Ok(Some(summary.to_string()))
    }
}

/// Stringify a JSON value and truncate to `max_len` chars, suffixing with
/// `...` when cut. Mirrors TS `truncateJson`.
fn truncate_json(v: &Value, max_len: usize) -> String {
    let s = match serde_json::to_string(v) {
        Ok(s) => s,
        Err(_) => return "[unable to serialize]".into(),
    };
    if s.len() <= max_len {
        return s;
    }
    let cut_at = max_len.saturating_sub(3);
    let mut cut = cut_at.min(s.len());
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}...", &s[..cut])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn empty_batch_returns_none() {
        let out = generate_tool_use_summary(&[], None, CancellationToken::new())
            .await
            .unwrap();
        assert!(out.is_none());
    }

    #[tokio::test]
    async fn no_model_registered_returns_none() {
        let tools = vec![ToolInfo {
            name: "Read".into(),
            input: json!({"path": "/tmp/x"}),
            output: json!({"content": "hi"}),
        }];
        // Secondary model is not installed in the test harness.
        let out =
            generate_tool_use_summary(&tools, Some("previous text"), CancellationToken::new())
                .await
                .unwrap();
        assert!(out.is_none());
    }

    #[test]
    fn truncate_json_short_passes_through() {
        let v = json!({"a": 1});
        let s = truncate_json(&v, 100);
        assert_eq!(s, "{\"a\":1}");
    }

    #[test]
    fn truncate_json_long_is_cut_with_ellipsis() {
        let big: String = "x".repeat(400);
        let v = json!({"big": big});
        let s = truncate_json(&v, 100);
        assert!(s.ends_with("..."));
        assert!(s.len() <= 100);
    }

    #[test]
    fn error_id_wired() {
        // Sanity: the error ID export compiles and is correct.
        assert_eq!(E_TOOL_USE_SUMMARY_GENERATION_FAILED, 344);
    }
}
