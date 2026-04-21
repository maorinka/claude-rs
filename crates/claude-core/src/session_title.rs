//! Session title generation — prompt only.
//!
//! Port of TS `utils/sessionTitle.ts:56-68`. The caller (a
//! Haiku-backed LLM query that runs post-session to name the
//! session for display) is not yet ported; this module exposes
//! the prompt constant so when the caller lands it can use the
//! verbatim TS string.

/// Prompt sent to the session-title LLM query. Instructs the
/// model to return JSON `{"title": "..."}` with a concise
/// sentence-case title.
///
/// Port of TS `utils/sessionTitle.ts:56-68` `SESSION_TITLE_PROMPT`.
pub const SESSION_TITLE_PROMPT: &str = r#"Generate a concise, sentence-case title (3-7 words) that captures the main topic or goal of this coding session. The title should be clear enough that the user recognizes the session in a list. Use sentence case: capitalize only the first word and proper nouns.

Return JSON with a single "title" field.

Good examples:
{"title": "Fix login button on mobile"}
{"title": "Add OAuth authentication"}
{"title": "Debug failing CI tests"}
{"title": "Refactor API client error handling"}

Bad (too vague): {"title": "Code changes"}
Bad (too long): {"title": "Investigate and fix the issue where the login button does not respond on mobile devices"}
Bad (wrong case): {"title": "Fix Login Button On Mobile"}"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_contains_ts_anchor_phrases() {
        // Verify the verbatim-from-TS content is present.
        assert!(SESSION_TITLE_PROMPT.contains("Generate a concise, sentence-case title"));
        assert!(SESSION_TITLE_PROMPT.contains("sentence case"));
        assert!(SESSION_TITLE_PROMPT.contains(r#"{"title": "Fix login button on mobile"}"#));
    }

    #[test]
    fn prompt_is_nonempty_ascii() {
        assert!(!SESSION_TITLE_PROMPT.is_empty());
        assert!(SESSION_TITLE_PROMPT.is_ascii());
    }
}
