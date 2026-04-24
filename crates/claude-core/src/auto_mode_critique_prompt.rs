//! Auto-mode classifier-rules critique prompts.
//!
//! Ports TS `cli/handlers/autoMode.ts:49` (system prompt) and
//! `:121-133` (user message template). TS runs a Sonnet query
//! via `claude auto-mode critique` to review the user's custom
//! auto-mode `allow` / `soft_deny` / `environment` rules for
//! clarity / completeness / conflicts.
//!
//! The CLI `auto-mode critique` subcommand isn't yet wired in
//! Rust; permissions infrastructure exists
//! (`crates/claude-core/src/permissions/`) but the Sonnet-backed
//! reviewer is deferred. This module exposes the two
//! strings verbatim for when the caller lands.

/// System prompt sent to the critique Sonnet query. Verbatim
/// from TS `cli/handlers/autoMode.ts:49`.
pub const CRITIQUE_SYSTEM_PROMPT: &str = "You are an expert reviewer of auto mode classifier rules for Claude Code.\n\
\n\
Claude Code has an \"auto mode\" that uses an AI classifier to decide whether tool calls should be auto-approved or require user confirmation. Users can write custom rules in three categories:\n\
\n\
- **allow**: Actions the classifier should auto-approve\n\
- **soft_deny**: Actions the classifier should block (require user confirmation)\n\
- **environment**: Context about the user's setup that helps the classifier make decisions\n\
\n\
Your job is to critique the user's custom rules for clarity, completeness, and potential issues. The classifier is an LLM that reads these rules as part of its system prompt.\n\
\n\
For each rule, evaluate:\n\
1. **Clarity**: Is the rule unambiguous? Could the classifier misinterpret it?\n\
2. **Completeness**: Are there gaps or edge cases the rule doesn't cover?\n\
3. **Conflicts**: Do any of the rules conflict with each other?\n\
4. **Actionability**: Is the rule specific enough for the classifier to act on?\n\
\n\
Be concise and constructive. Only comment on rules that could be improved. If all rules look good, say so.";

/// Build the user-message for the critique query. Wraps the
/// classifier system prompt in `<classifier_system_prompt>`
/// tags, then appends the user's custom-rules summary and a
/// final "please critique" instruction. Verbatim from TS
/// `cli/handlers/autoMode.ts:121-131`.
pub fn critique_user_message(classifier_prompt: &str, user_rules_summary: &str) -> String {
    format!(
        "Here is the full classifier system prompt that the auto mode classifier receives:\n\n\
         <classifier_system_prompt>\n\
         {classifier_prompt}\n\
         </classifier_system_prompt>\n\n\
         Here are the user's custom rules that REPLACE the corresponding default sections:\n\n\
         {user_rules_summary}\n\
         Please critique these custom rules."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_prompt_names_the_three_categories() {
        assert!(CRITIQUE_SYSTEM_PROMPT.contains("**allow**"));
        assert!(CRITIQUE_SYSTEM_PROMPT.contains("**soft_deny**"));
        assert!(CRITIQUE_SYSTEM_PROMPT.contains("**environment**"));
    }

    #[test]
    fn system_prompt_lists_evaluation_axes() {
        assert!(CRITIQUE_SYSTEM_PROMPT.contains("**Clarity**"));
        assert!(CRITIQUE_SYSTEM_PROMPT.contains("**Completeness**"));
        assert!(CRITIQUE_SYSTEM_PROMPT.contains("**Conflicts**"));
        assert!(CRITIQUE_SYSTEM_PROMPT.contains("**Actionability**"));
    }

    #[test]
    fn user_message_wraps_classifier_prompt() {
        let m = critique_user_message("<classifier system>", "- allow: do X\n- soft_deny: do Y\n");
        assert!(m.contains(
            "<classifier_system_prompt>\n<classifier system>\n</classifier_system_prompt>"
        ));
        assert!(m.contains("- allow: do X"));
        assert!(m.ends_with("Please critique these custom rules."));
    }
}
