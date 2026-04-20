//! Width-aware string truncation and soft-wrapping.
//!
//! Port of TS `utils/truncate.ts:1-179`.
//!
//! All functions measure in *terminal columns* (Unicode East Asian
//! Width), not byte count or char count, so emoji and CJK render
//! correctly under a fixed budget. Splits at grapheme boundaries via
//! `unicode-segmentation` so ZWJ sequences and combining marks never
//! break mid-cluster.
//!
//! TS delegates width calculation to the ink/`string-width` package;
//! Rust uses `unicode-width::UnicodeWidthStr`. Both spec out from UAX
//! #11 + the extended East Asian Width table, so the measurements
//! agree in practice.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

const ELLIPSIS: &str = "…";

fn width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// Truncate `text` to fit within `max_width` columns. Appends `…`
/// when truncation occurs. Matches TS `truncateToWidth`.
pub fn truncate_to_width(text: &str, max_width: usize) -> String {
    if width(text) <= max_width {
        return text.to_owned();
    }
    if max_width <= 1 {
        return ELLIPSIS.to_owned();
    }
    let mut w = 0usize;
    let mut out = String::new();
    for seg in text.graphemes(true) {
        let sw = width(seg);
        if w + sw > max_width - 1 {
            break;
        }
        out.push_str(seg);
        w += sw;
    }
    out.push_str(ELLIPSIS);
    out
}

/// Truncate from the start, keeping the tail. Prepends `…`. Matches
/// TS `truncateStartToWidth`.
pub fn truncate_start_to_width(text: &str, max_width: usize) -> String {
    if width(text) <= max_width {
        return text.to_owned();
    }
    if max_width <= 1 {
        return ELLIPSIS.to_owned();
    }
    let segs: Vec<&str> = text.graphemes(true).collect();
    let mut w = 0usize;
    let mut start_idx = segs.len();
    for i in (0..segs.len()).rev() {
        let sw = width(segs[i]);
        if w + sw > max_width - 1 {
            break;
        }
        w += sw;
        start_idx = i;
    }
    let mut out = String::with_capacity(text.len());
    out.push_str(ELLIPSIS);
    for seg in &segs[start_idx..] {
        out.push_str(seg);
    }
    out
}

/// Truncate to fit width WITHOUT appending an ellipsis. Callers use
/// this when they add their own separator (e.g. middle-truncation).
/// Matches TS `truncateToWidthNoEllipsis`.
pub fn truncate_to_width_no_ellipsis(text: &str, max_width: usize) -> String {
    if width(text) <= max_width {
        return text.to_owned();
    }
    if max_width == 0 {
        return String::new();
    }
    let mut w = 0usize;
    let mut out = String::new();
    for seg in text.graphemes(true) {
        let sw = width(seg);
        if w + sw > max_width {
            break;
        }
        out.push_str(seg);
        w += sw;
    }
    out
}

/// Middle-truncate a path: keep directory context + full filename.
/// `"src/components/.../MyComponent.tsx"`.
///
/// Matches TS `truncatePathMiddle`. `max_length` is width, not byte
/// length — names the parameter the TS way for call-site parity.
pub fn truncate_path_middle(path: &str, max_length: usize) -> String {
    if width(path) <= max_length {
        return path.to_owned();
    }
    if max_length == 0 {
        return ELLIPSIS.to_owned();
    }
    if max_length < 5 {
        return truncate_to_width(path, max_length);
    }

    // TS uses `/` — Rust mirrors that exact char. Paths on Windows get
    // split on `/` too because that's what TS does; callers that need
    // `\` handling are expected to pass posix-normalised paths (we
    // already port `normalize_path_for_config_key` in `path_utils`).
    let last_slash = path.rfind('/');
    let filename = match last_slash {
        Some(i) => &path[i..],
        None => path,
    };
    let directory = match last_slash {
        Some(i) => &path[..i],
        None => "",
    };
    let filename_width = width(filename);

    if filename_width + 1 >= max_length {
        return truncate_start_to_width(path, max_length);
    }

    // Result layout: directory + `…` + filename.
    let available_for_dir = max_length - 1 - filename_width;
    if available_for_dir == 0 {
        return truncate_start_to_width(filename, max_length);
    }

    let truncated_dir = truncate_to_width_no_ellipsis(directory, available_for_dir);
    let mut out = String::with_capacity(truncated_dir.len() + ELLIPSIS.len() + filename.len());
    out.push_str(&truncated_dir);
    out.push_str(ELLIPSIS);
    out.push_str(filename);
    out
}

/// Single-entry truncator. Matches TS `truncate(str, maxWidth,
/// singleLine)`. When `single_line` is `true`, also cuts at the first
/// newline; the TS "append `…` if the truncated line would still fit"
/// branch is preserved verbatim since the extra width needs to leave
/// room for the ellipsis.
pub fn truncate(text: &str, max_width: usize, single_line: bool) -> String {
    if single_line {
        if let Some(nl) = text.find('\n') {
            let line = &text[..nl];
            if width(line) + 1 > max_width {
                return truncate_to_width(line, max_width);
            }
            return format!("{line}{ELLIPSIS}");
        }
    }
    if width(text) <= max_width {
        return text.to_owned();
    }
    truncate_to_width(text, max_width)
}

/// Greedy wrap at grapheme boundaries. Never breaks inside a cluster.
/// Does NOT attempt word-boundary wrapping — matches TS `wrapText`.
pub fn wrap_text(text: &str, width_cols: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;

    for seg in text.graphemes(true) {
        let sw = width(seg);
        if current_width + sw <= width_cols {
            current.push_str(seg);
            current_width += sw;
        } else {
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
            }
            current.push_str(seg);
            current_width = sw;
        }
    }

    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_to_width_within_budget_is_identity() {
        assert_eq!(truncate_to_width("hello", 10), "hello");
    }

    #[test]
    fn truncate_to_width_appends_ellipsis() {
        assert_eq!(truncate_to_width("hello world", 8), "hello w…");
    }

    #[test]
    fn truncate_to_width_with_width_one_returns_ellipsis() {
        assert_eq!(truncate_to_width("hello", 1), ELLIPSIS);
        assert_eq!(truncate_to_width("hello", 0), ELLIPSIS);
    }

    #[test]
    fn truncate_to_width_respects_grapheme_clusters() {
        // Family emoji is 1 grapheme, width 2. `👨‍👩‍👧hello` at width 5
        // should keep the family + some text.
        let input = "👨‍👩‍👧hello";
        let out = truncate_to_width(input, 5);
        // Must end with ellipsis and contain the family as a whole.
        assert!(out.ends_with(ELLIPSIS));
        assert!(out.starts_with("👨‍👩‍👧"));
    }

    #[test]
    fn truncate_start_keeps_tail() {
        assert_eq!(truncate_start_to_width("abcdefghij", 5), "…ghij");
    }

    #[test]
    fn truncate_start_tiny_width() {
        assert_eq!(truncate_start_to_width("abc", 1), ELLIPSIS);
    }

    #[test]
    fn truncate_no_ellipsis_cuts_clean() {
        assert_eq!(truncate_to_width_no_ellipsis("hello world", 5), "hello");
    }

    #[test]
    fn truncate_path_middle_preserves_filename() {
        let p = "src/components/deeply/nested/folder/MyComponent.tsx";
        let out = truncate_path_middle(p, 30);
        assert!(out.ends_with("MyComponent.tsx"), "got {out}");
        assert!(out.contains(ELLIPSIS), "got {out}");
        assert!(width(&out) <= 30, "width {} > 30 in {out}", width(&out));
    }

    #[test]
    fn truncate_path_middle_fits_returns_identity() {
        assert_eq!(
            truncate_path_middle("short/path.ts", 100),
            "short/path.ts"
        );
    }

    #[test]
    fn truncate_path_middle_huge_filename_falls_back_to_start() {
        let p = "a/really_really_long_filename_that_exceeds_the_budget.ts";
        let out = truncate_path_middle(p, 20);
        // Filename alone is too long; falls back to start-truncation, which
        // prepends `…`.
        assert!(out.starts_with(ELLIPSIS));
        assert!(width(&out) <= 20);
    }

    #[test]
    fn truncate_single_line_cuts_at_newline() {
        assert_eq!(truncate("first\nsecond\nthird", 100, true), "first…");
    }

    #[test]
    fn truncate_single_line_respects_width_cap() {
        // The "first" line plus ellipsis would exceed width 3 — must
        // re-truncate.
        let out = truncate("first\nsecond", 3, true);
        assert!(width(&out) <= 3);
        assert!(out.ends_with(ELLIPSIS));
    }

    #[test]
    fn truncate_multi_line_single_line_false_wraps_in_width() {
        // Input narrower than budget: identity.
        assert_eq!(truncate("hi there", 20, false), "hi there");
        // Wider: normal truncate.
        let out = truncate("hi there friend", 8, false);
        assert!(out.ends_with(ELLIPSIS));
        assert!(width(&out) <= 8);
    }

    #[test]
    fn wrap_text_greedy_wraps_at_width() {
        let out = wrap_text("abcdefghij", 3);
        assert_eq!(out, vec!["abc", "def", "ghi", "j"]);
    }

    #[test]
    fn wrap_text_respects_grapheme_clusters() {
        // 2-column emoji gets its own line when budget is 1.
        let out = wrap_text("a😀b", 1);
        // `a`, then emoji (can't fit in width 1 on its own line either
        // — but we push it anyway since it's one grapheme), then `b`.
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn wrap_text_empty() {
        assert_eq!(wrap_text("", 5), Vec::<String>::new());
    }
}
