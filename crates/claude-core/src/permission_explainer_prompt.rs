//! Permission-explainer system prompt — constant only.
//!
//! Port of TS `utils/permissions/permissionExplainer.ts:43`.
//! TS fires a Haiku query via `explain_command` tool-call to
//! generate the human-readable explanation shown in the
//! permission-prompt dialog when a Bash command needs approval.
//! The Rust permission evaluator doesn't call an LLM today —
//! permission decisions surface the raw command instead. This
//! module exposes the verbatim system prompt so when the
//! explainer lands it runs with identical framing.

/// The one-line system prompt for the explainer Haiku query.
/// Port of TS `permissionExplainer.ts:43`.
pub const PERMISSION_EXPLAINER_SYSTEM_PROMPT: &str =
    "Analyze shell commands and explain what they do, why you're running them, and potential risks.";

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
}
