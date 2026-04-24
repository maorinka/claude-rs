//! Port of `src/constants/toolLimits.ts`.
//!
//! System-wide caps for tool result sizes. Individual tools may declare
//! a lower `maxResultSizeChars`, but these constants act as the
//! session-level ceiling.

/// Default maximum characters in a tool result before it's persisted to
/// disk and the model receives a preview + file path instead.
pub const DEFAULT_MAX_RESULT_SIZE_CHARS: usize = 50_000;

/// Approximate tokens-per-tool-result cap. TS comment: "approximately
/// 400KB of text assuming ~4 bytes per token".
pub const MAX_TOOL_RESULT_TOKENS: usize = 100_000;

/// Conservative bytes-per-token estimate used to convert byte sizes to
/// token counts.
pub const BYTES_PER_TOKEN: usize = 4;

/// Derived byte cap: `MAX_TOOL_RESULT_TOKENS * BYTES_PER_TOKEN`.
pub const MAX_TOOL_RESULT_BYTES: usize = MAX_TOOL_RESULT_TOKENS * BYTES_PER_TOKEN;

/// Per-message aggregate cap — sum of tool_result block sizes within one
/// turn's user message. Prevents N parallel tools from each hitting the
/// per-tool cap and collectively exploding the context.
pub const MAX_TOOL_RESULTS_PER_MESSAGE_CHARS: usize = 200_000;

/// Summary-display truncation cap used by grouped agent rendering.
pub const TOOL_SUMMARY_MAX_LENGTH: usize = 50;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_cap_is_derived() {
        assert_eq!(
            MAX_TOOL_RESULT_BYTES,
            MAX_TOOL_RESULT_TOKENS * BYTES_PER_TOKEN
        );
    }

    #[test]
    fn per_message_cap_exceeds_per_tool_cap() {
        assert_eq!(
            MAX_TOOL_RESULTS_PER_MESSAGE_CHARS.cmp(&DEFAULT_MAX_RESULT_SIZE_CHARS),
            std::cmp::Ordering::Greater
        );
    }
}
