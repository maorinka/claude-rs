//! Memory-selection system prompt — constant only.
//!
//! Port of TS `memdir/findRelevantMemories.ts:18`
//! `SELECT_MEMORIES_SYSTEM_PROMPT`. The Rust memdir system
//! exists (`memdir/scan.rs`, `memdir/entrypoint.rs`) but the
//! Sonnet-backed "pick up to 5 relevant memories" query is not
//! yet wired — this module exposes the system prompt so when
//! that caller lands it uses identical framing.

/// System prompt sent to Sonnet for memory selection. Tells the
/// model to pick up to 5 memory files whose names/descriptions
/// are clearly useful for the incoming user query.
///
/// Port of TS `memdir/findRelevantMemories.ts:18`.
pub const SELECT_MEMORIES_SYSTEM_PROMPT: &str = "You are selecting memories that will be useful to Claude Code as it processes a user's query. You will be given the user's query and a list of available memory files with their filenames and descriptions.

Return a list of filenames for the memories that will clearly be useful to Claude Code as it processes the user's query (up to 5). Only include memories that you are certain will be helpful based on their name and description.
- If you are unsure if a memory will be useful in processing the user's query, then do not include it in your list. Be selective and discerning.
- If there are no memories in the list that would clearly be useful, feel free to return an empty list.
- If a list of recently-used tools is provided, do not select memories that are usage reference or API documentation for those tools (Claude Code is already exercising them). DO still select memories containing warnings, gotchas, or known issues about those tools — active use is exactly when those matter.
";

/// Build the user message sent alongside
/// [`SELECT_MEMORIES_SYSTEM_PROMPT`]. Port of TS
/// `memdir/findRelevantMemories.ts:103`.
///
/// - `query` is the user's incoming request text.
/// - `manifest` is the pre-rendered `- filename: description`
///   list of candidate memories.
/// - `tools_section` is a pre-rendered list of recently-used
///   tools (the TS function prefixes it with two blank lines +
///   `Recently used tools:` when non-empty; the caller owns
///   that formatting). Pass an empty string to omit.
pub fn select_memories_user_message(query: &str, manifest: &str, tools_section: &str) -> String {
    format!("Query: {query}\n\nAvailable memories:\n{manifest}{tools_section}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_contains_ts_anchor_phrases() {
        assert!(SELECT_MEMORIES_SYSTEM_PROMPT.contains("You are selecting memories"));
        assert!(SELECT_MEMORIES_SYSTEM_PROMPT.contains("up to 5"));
        assert!(SELECT_MEMORIES_SYSTEM_PROMPT.contains("active use is exactly when those matter"));
    }

    #[test]
    fn user_message_has_ts_header_order() {
        let msg = select_memories_user_message(
            "how do I debug the parser",
            "- parser_notes.md: grammar edge cases",
            "",
        );
        assert!(msg.starts_with("Query: how do I debug the parser"));
        assert!(msg.contains("Available memories:\n- parser_notes.md: grammar edge cases"));
    }

    #[test]
    fn user_message_appends_tools_section_verbatim() {
        let tools = "\n\nRecently used tools:\n- Read\n- Grep";
        let msg = select_memories_user_message("q", "- a.md: x", tools);
        assert!(msg.ends_with("- Read\n- Grep"));
        assert!(msg.contains("Recently used tools:"));
    }

    #[test]
    fn user_message_without_tools_ends_at_manifest() {
        let msg = select_memories_user_message("q", "- a.md: x", "");
        assert!(msg.ends_with("- a.md: x"));
    }
}
