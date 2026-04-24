//! Per-tool-result truncation for the query loop.
//!
//! Minimal port of the TS `applyToolResultBudget` surface. The TS module
//! (utils/toolResultStorage.ts) also handles persistent content-replacement
//! records, re-apply on resume, and fork-subagent gap-fill; porting that
//! full system requires transcript persistence we haven't wired yet.
//! This patch provides the hot-path behaviour: a hard per-result cap so a
//! 10 MB grep dump doesn't blow the context window.

/// Default maximum bytes per tool result sent back to the model.
/// TS uses a configurable budget; until we surface that config we pick a
/// conservative default that matches tool-specific caps already in place
/// (Bash MAX_OUTPUT_CHARS, WebFetch truncation both hover around this).
pub const DEFAULT_MAX_TOOL_RESULT_BYTES: usize = 100_000;

/// If `content` exceeds `max_bytes`, truncate at a UTF-8 boundary and
/// append a marker. Leaves shorter inputs untouched.
pub fn truncate_tool_result(content: &str, max_bytes: usize) -> String {
    if content.len() <= max_bytes {
        return content.to_string();
    }
    let mut cut = max_bytes;
    while cut > 0 && !content.is_char_boundary(cut) {
        cut -= 1;
    }
    format!(
        "{}\n\n[tool result truncated: original was {} bytes, kept {} bytes]",
        &content[..cut],
        content.len(),
        cut
    )
}

/// Convenience wrapper: apply the default cap.
pub fn apply_default_budget(content: &str) -> String {
    truncate_tool_result(content, DEFAULT_MAX_TOOL_RESULT_BYTES)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_passes_through() {
        assert_eq!(truncate_tool_result("hi", 100), "hi");
    }

    #[test]
    fn long_is_truncated_with_marker() {
        let s = "x".repeat(200);
        let out = truncate_tool_result(&s, 50);
        assert!(out.starts_with(&"x".repeat(50)));
        assert!(out.contains("truncated"));
        assert!(out.contains("200 bytes"));
    }

    #[test]
    fn utf8_boundary_safe() {
        // 4-byte codepoint is U+1F600 (grinning face). Each char is 4 bytes,
        // so a cap that falls inside a char must backtrack.
        let s: String = std::iter::repeat_n('\u{1F600}', 50).collect();
        let out = truncate_tool_result(&s, 51); // between 12th and 13th char
        assert!(out.starts_with("\u{1F600}"));
    }

    #[test]
    fn default_budget_respects_limit() {
        let huge = "y".repeat(DEFAULT_MAX_TOOL_RESULT_BYTES * 2);
        let out = apply_default_budget(&huge);
        assert!(out.len() < DEFAULT_MAX_TOOL_RESULT_BYTES + 200);
    }
}
