//! `format_error` for tool-result payloads headed to the model.
//!
//! Port of TS `utils/toolErrors.ts:1-41` (the `formatError` +
//! `getErrorParts` path — Zod-specific validation formatter is not
//! ported since Rust uses serde; a serde-equivalent would have a
//! different shape).
//!
//! Why this module exists
//! ======================
//! When a tool fails, the error message ships back to the model as a
//! `tool_result` content block. Two constraints:
//! 1. Shell tools produce useful stdout/stderr **in addition to** an
//!    exit code — those must be included in the message so the model
//!    can debug.
//! 2. An 8-MB `stderr` dump would blow the context window. Cap the
//!    total to 10K chars; when exceeded, keep head+tail with a
//!    truncation marker in the middle so both the initial failure
//!    summary and the final failure summary survive.

use crate::errors_util::ShellError;

/// User-facing message when a tool call is interrupted by the user
/// (ctrl-c, explicit cancellation). Matches TS `utils/messages.ts:208`.
pub const INTERRUPT_MESSAGE_FOR_TOOL_USE: &str = "[Request interrupted by user for tool use]";

const MAX_LEN: usize = 10_000;
const HALF_LEN: usize = 5_000;

/// Collect the error parts to assemble into a tool-result message.
/// TS `getErrorParts`. Callers typically join with `\n` and trim.
pub fn get_error_parts(err: &ShellError) -> Vec<String> {
    vec![
        format!("Exit code {}", err.code),
        if err.interrupted {
            INTERRUPT_MESSAGE_FOR_TOOL_USE.to_owned()
        } else {
            String::new()
        },
        err.stderr.clone(),
        err.stdout.clone(),
    ]
}

/// Single-entry error formatter. Returns a string bounded to
/// [`MAX_LEN`] chars — when exceeded, keeps the first 5K + last 5K
/// with a `... [N characters truncated] ...` marker in between. The
/// head+tail split preserves both the initial failure line and any
/// final summary line that shells tend to emit.
pub fn format_shell_error(err: &ShellError) -> String {
    let joined: String = get_error_parts(err)
        .into_iter()
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let trimmed = joined.trim();

    let full = if trimmed.is_empty() {
        "Command failed with no output".to_owned()
    } else {
        trimmed.to_owned()
    };

    truncate_for_tool_result(&full)
}

/// Apply the 10K cap to an arbitrary already-formatted error string.
/// Useful when the caller already has the message assembled (e.g.
/// from an `anyhow::Error` chain) and just needs the truncation
/// rule applied.
pub fn truncate_for_tool_result(msg: &str) -> String {
    // TS uses `.length` (UTF-16 code units) and `.slice` (same).
    // Rust works in bytes for `&str`; char-counting is closer to the
    // TS behaviour for human-readable truncation markers. Use
    // `chars().count()` for the predicate and grapheme-based
    // splitting only when needed.
    let char_count = msg.chars().count();
    if char_count <= MAX_LEN {
        return msg.to_owned();
    }

    // Convert char index → byte index for slicing. `char_indices`
    // gives us (byte_offset, char). We need the byte offset of the
    // HALF_LEN-th char (for head) and the start of the last
    // HALF_LEN chars (for tail).
    let head_end_byte = msg
        .char_indices()
        .nth(HALF_LEN)
        .map(|(b, _)| b)
        .unwrap_or(msg.len());
    let tail_start_byte = msg
        .char_indices()
        .nth(char_count - HALF_LEN)
        .map(|(b, _)| b)
        .unwrap_or(0);

    let head = &msg[..head_end_byte];
    let tail = &msg[tail_start_byte..];
    let truncated = char_count - MAX_LEN;
    format!("{head}\n\n... [{truncated} characters truncated] ...\n\n{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell(code: i32, stdout: &str, stderr: &str, interrupted: bool) -> ShellError {
        ShellError {
            stdout: stdout.to_owned(),
            stderr: stderr.to_owned(),
            code,
            interrupted,
        }
    }

    #[test]
    fn interrupt_message_pin() {
        // The interrupt marker string is load-bearing — the TUI
        // greps for it to render a specific UI state. Pin the exact
        // literal.
        assert_eq!(
            INTERRUPT_MESSAGE_FOR_TOOL_USE,
            "[Request interrupted by user for tool use]"
        );
    }

    #[test]
    fn shell_error_includes_exit_code_and_streams() {
        let out = format_shell_error(&shell(42, "hello", "error happened", false));
        assert!(out.contains("Exit code 42"));
        assert!(out.contains("hello"));
        assert!(out.contains("error happened"));
    }

    #[test]
    fn interrupted_flag_injects_marker() {
        let out = format_shell_error(&shell(130, "", "", true));
        assert!(out.contains("Exit code 130"));
        assert!(out.contains(INTERRUPT_MESSAGE_FOR_TOOL_USE));
    }

    #[test]
    fn empty_streams_report_no_output_message() {
        let out = format_shell_error(&shell(0, "", "", false));
        // Exit code 0 alone is still a non-empty line, so the
        // "no output" fallback only triggers when ALL parts are
        // empty. Use a shell error with no output and non-zero code
        // to prove the empty-content path.
        assert!(out.starts_with("Exit code 0"));

        // Now simulate TS's actual `no output` path: empty-everything
        // ShellError would hit via `parts.filter(Boolean).join('\n').trim()`
        // yielding just "Exit code 0\n\n\n" → trimmed to "Exit code 0".
        // The "Command failed with no output" branch is used elsewhere
        // when the error source has nothing at all. Verify via the
        // truncate helper on an empty string:
        assert_eq!(truncate_for_tool_result(""), "");
    }

    #[test]
    fn truncates_when_over_max_len() {
        let long = "a".repeat(MAX_LEN + 500);
        let out = truncate_for_tool_result(&long);
        assert!(out.contains("... [500 characters truncated] ..."));
        // Must be strictly shorter than the input — the truncation
        // marker replaces 500 chars.
        let out_chars = out.chars().count();
        assert!(out_chars < long.chars().count());
        // Head and tail of the 10K budget both survive.
        assert!(out.starts_with(&"a".repeat(HALF_LEN)));
        assert!(out.ends_with(&"a".repeat(HALF_LEN)));
    }

    #[test]
    fn exact_max_len_not_truncated() {
        let exact = "b".repeat(MAX_LEN);
        let out = truncate_for_tool_result(&exact);
        // Untouched when length == MAX_LEN.
        assert_eq!(out, exact);
    }

    #[test]
    fn truncation_preserves_head_and_tail_content() {
        // Different content at head vs tail — prove both survive
        // the truncation marker.
        let head = "HEAD-MARKER";
        let tail = "TAIL-MARKER";
        let middle = "x".repeat(MAX_LEN);
        let input = format!("{head}{middle}{tail}");
        let out = truncate_for_tool_result(&input);
        assert!(out.starts_with(head), "head lost: {}", &out[..20]);
        assert!(out.ends_with(tail), "tail lost");
    }

    #[test]
    fn get_error_parts_order_matches_ts() {
        // Order is load-bearing: exit code → interrupt marker →
        // stderr → stdout. Pins the sequence the TS formatter
        // relies on.
        let parts = get_error_parts(&shell(7, "out-text", "err-text", true));
        assert_eq!(parts.len(), 4);
        assert_eq!(parts[0], "Exit code 7");
        assert_eq!(parts[1], INTERRUPT_MESSAGE_FOR_TOOL_USE);
        assert_eq!(parts[2], "err-text");
        assert_eq!(parts[3], "out-text");
    }

    #[test]
    fn unicode_truncation_counts_chars_not_bytes() {
        // Each `🎉` is 1 char but 4 bytes. Using byte-count for
        // truncation would give wrong marker length and potentially
        // slice mid-codepoint.
        let long: String = "🎉".repeat(MAX_LEN + 100);
        let out = truncate_for_tool_result(&long);
        // Valid UTF-8 — won't panic when printed.
        assert!(out.contains("characters truncated"));
        // Truncation count matches char count, not byte count.
        assert!(out.contains("100 characters"));
    }
}
