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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_contains_ts_anchor_phrases() {
        assert!(SELECT_MEMORIES_SYSTEM_PROMPT.contains("You are selecting memories"));
        assert!(SELECT_MEMORIES_SYSTEM_PROMPT.contains("up to 5"));
        assert!(SELECT_MEMORIES_SYSTEM_PROMPT.contains("active use is exactly when those matter"));
    }
}
