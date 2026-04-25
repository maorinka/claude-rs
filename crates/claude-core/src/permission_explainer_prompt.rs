//! Permission-explainer system prompt + Haiku caller.
//!
//! Port of TS `utils/permissions/permissionExplainer.ts:43-180`.
//! TS fires a Haiku query via `explain_command` tool-call to
//! generate the human-readable explanation shown in the
//! permission-prompt dialog when a Bash command needs approval.

use anyhow::Result;
use tokio_util::sync::CancellationToken;

use crate::secondary_model;
use crate::system_prompt_extensions::permission_explainer_user_prompt;

/// The one-line system prompt for the explainer Haiku query.
/// Port of TS `permissionExplainer.ts:43`.
pub const PERMISSION_EXPLAINER_SYSTEM_PROMPT: &str =
    "Analyze shell commands and explain what they do, why you're running them, and potential risks.";

/// Generate a one-paragraph explanation of a tool invocation for
/// surfacing in the permission-prompt UI. Calls the registered
/// secondary model (Haiku) with the verbatim TS framing. Returns
/// `Ok(None)` when no secondary model is registered so callers can
/// fall back to the raw command without erroring out.
///
/// `tool_description` and `conversation_context` may be empty —
/// the system_prompt_extensions builder elides those segments
/// exactly as TS does.
pub async fn explain_command(
    tool_name: &str,
    tool_description: &str,
    formatted_input: &str,
    conversation_context: &str,
    cancel: CancellationToken,
) -> Result<Option<String>> {
    let Some(model) = secondary_model::get_global() else {
        return Ok(None);
    };
    let user = permission_explainer_user_prompt(
        tool_name,
        tool_description,
        formatted_input,
        conversation_context,
    );
    let composed = format!("{PERMISSION_EXPLAINER_SYSTEM_PROMPT}\n\n{user}");
    let response = model.summarize(&composed, cancel).await?;
    let trimmed = response.trim();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_prompt_matches_ts_verbatim() {
        assert_eq!(
            PERMISSION_EXPLAINER_SYSTEM_PROMPT,
            "Analyze shell commands and explain what they do, why you're running them, and potential risks."
        );
    }

    #[tokio::test]
    async fn returns_none_when_no_secondary_model_registered() {
        // We do not call set_global() here, so get_global() yields None.
        // The function should gracefully return Ok(None).
        let out = explain_command(
            "Bash",
            "Run shell commands",
            "command: 'ls -la'",
            "",
            CancellationToken::new(),
        )
        .await
        .expect("must not error when no model registered");
        assert!(out.is_none());
    }
}
