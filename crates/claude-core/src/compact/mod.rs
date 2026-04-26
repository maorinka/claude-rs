pub mod compactor;
pub mod prompt;

/// User-facing warning shown when the TUI tries to edit or reference a
/// message that has been removed from the active context by compaction.
/// Verbatim port of TS `src/screens/REPL.tsx:4928` — the exact wording
/// matches what users see in Claude Code so behaviour is identical.
/// The TUI call site is not yet wired; exposing the constant keeps the
/// wording in one place for when the selector lands.
pub const SNIPPED_OR_PRECOMPACT_WARNING: &str =
    "That message is no longer in the active context (snipped or pre-compact). Choose a more recent message.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snipped_warning_matches_ts_wording() {
        // Must match TS src/screens/REPL.tsx:4928 verbatim — users see
        // this exact string.
        assert_eq!(
            SNIPPED_OR_PRECOMPACT_WARNING,
            "That message is no longer in the active context (snipped or pre-compact). Choose a more recent message."
        );
    }
}
