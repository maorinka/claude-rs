//! Feedback-dialog GitHub-issue title generation — prompt only.
//!
//! Port of TS `components/Feedback.tsx:450` `generateTitle`.
//! The Rust TUI `FeedbackDialog` captures ratings + description
//! locally but doesn't yet fire a Haiku query to generate a
//! GitHub-issue title. This module exposes the verbatim TS
//! system prompt so when that call lands it runs with identical
//! framing.

/// Joined system prompt for the GitHub-issue-title Haiku query.
/// TS assembles this as an array-of-strings passed to
/// `asSystemPrompt([...])`; the Rust port joins them with `\n\n`
/// at build time so callers pass a single owned `String`.
pub const FEEDBACK_TITLE_SYSTEM_PROMPT_LINES: &[&str] = &[
    "Generate a concise, technical issue title (max 80 chars) for a public GitHub issue based on this bug report for Claude Code.",
    "Claude Code is an agentic coding CLI based on the Anthropic API.",
    "The title should:",
    "- Include the type of issue [Bug] or [Feature Request] as the first thing in the title",
    "- Be concise, specific and descriptive of the actual problem",
    "- Use technical terminology appropriate for a software issue",
    "- For error messages, extract the key error (e.g., \"Missing Tool Result Block\" rather than the full message)",
    "- Be direct and clear for developers to understand the problem",
    "- If you cannot determine a clear issue, use \"Bug Report: [brief description]\"",
    "- Any LLM API errors are from the Anthropic API, not from any other model provider",
    "Your response will be directly used as the title of the Github issue, and as such should not contain any other commentary or explaination",
    "Examples of good titles include: \"[Bug] Auto-Compact triggers to soon\", \"[Bug] Anthropic API Error: Missing Tool Result Block\", \"[Bug] Error: Invalid Model Name for Opus\"",
];

/// Join `FEEDBACK_TITLE_SYSTEM_PROMPT_LINES` with `\n\n` into a
/// single system-prompt string ready to pass to the API.
pub fn feedback_title_system_prompt() -> String {
    FEEDBACK_TITLE_SYSTEM_PROMPT_LINES.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_prompt_is_nonempty() {
        let s = feedback_title_system_prompt();
        assert!(!s.is_empty());
        assert!(s.contains("Generate a concise, technical issue title"));
    }

    #[test]
    fn has_expected_guidelines() {
        let s = feedback_title_system_prompt();
        assert!(s.contains("[Bug] or [Feature Request]"));
        assert!(s.contains("max 80 chars"));
        assert!(s.contains("Missing Tool Result Block"));
    }

    #[test]
    fn examples_list_included() {
        let s = feedback_title_system_prompt();
        assert!(s.contains("[Bug] Auto-Compact triggers to soon"));
        assert!(s.contains("[Bug] Error: Invalid Model Name for Opus"));
    }
}
