//! Curly/straight quote preservation for FileEditTool.
//!
//! Port of the quote-normalising helpers in `src/tools/FileEditTool/utils.ts`:
//!   - `normalizeQuotes` (collapse curly → straight for matching)
//!   - `findActualString` (fall back to normalized matching so a model's
//!     straight-quote search string finds curly-quoted file content)
//!   - `preserveQuoteStyle` (when such a match happens, re-apply the
//!     file's curly-quote style to the replacement)
//!   - `stripTrailingWhitespace`
//!
//! Callers that want this behaviour wire it into edit.rs between the
//! "find the old_string" and "write the new_string" steps. The current
//! Rust edit.rs does literal `matches(old_string)` only; integrating
//! these helpers is a separate follow-up — this patch ships the
//! primitives so that integration can happen against real APIs.

pub const LEFT_SINGLE_CURLY_QUOTE: char = '\u{2018}';
pub const RIGHT_SINGLE_CURLY_QUOTE: char = '\u{2019}';
pub const LEFT_DOUBLE_CURLY_QUOTE: char = '\u{201C}';
pub const RIGHT_DOUBLE_CURLY_QUOTE: char = '\u{201D}';

/// Collapse all four curly-quote variants to their straight equivalents.
pub fn normalize_quotes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            LEFT_SINGLE_CURLY_QUOTE | RIGHT_SINGLE_CURLY_QUOTE => out.push('\''),
            LEFT_DOUBLE_CURLY_QUOTE | RIGHT_DOUBLE_CURLY_QUOTE => out.push('"'),
            other => out.push(other),
        }
    }
    out
}

/// Find `search` in `haystack`, trying an exact match first then a
/// normalised-quote match. On fallback hit, return the actual substring
/// from `haystack` at the matching offset (so the caller knows which
/// quote style to preserve). Returns None if neither variant matches.
///
/// Mirrors TS `findActualString`. Byte-offset semantics: we use the
/// normalized-file position to index back into the ORIGINAL file. That
/// works here because all four curly quotes are 3-byte UTF-8 (U+2018 =
/// 0xE2 0x80 0x98) while the straight counterparts are 1 byte, so the
/// offsets DIFFER by 2 bytes per replaced quote. To stay robust we walk
/// the original char-by-char to count how many bytes to skip until the
/// normalized index is reached.
pub fn find_actual_string<'a>(haystack: &'a str, search: &str) -> Option<&'a str> {
    if haystack.contains(search) {
        // Return the exact slice from haystack (same bytes as `search`).
        return haystack
            .find(search)
            .map(|idx| &haystack[idx..idx + search.len()]);
    }

    let normalized_file = normalize_quotes(haystack);
    let normalized_search = normalize_quotes(search);
    let n_idx = normalized_file.find(&normalized_search)?;

    // Map n_idx (byte offset in normalized_file) back to a byte offset
    // in haystack by walking chars in parallel.
    let mut orig_bytes = 0usize;
    let mut norm_bytes = 0usize;
    let mut orig_chars = haystack.char_indices().peekable();
    let mut norm_chars = normalized_file.char_indices().peekable();
    while norm_bytes < n_idx {
        let (_, nc) = norm_chars.next()?;
        let (_, oc) = orig_chars.next()?;
        orig_bytes += oc.len_utf8();
        norm_bytes += nc.len_utf8();
    }

    // Now walk until we've covered `normalized_search.len()` normalized bytes.
    let start = orig_bytes;
    let mut consumed_norm = 0usize;
    while consumed_norm < normalized_search.len() {
        let (_, nc) = norm_chars.next()?;
        let (_, oc) = orig_chars.next()?;
        orig_bytes += oc.len_utf8();
        consumed_norm += nc.len_utf8();
    }

    Some(&haystack[start..orig_bytes])
}

/// When `actual_old` came back from `find_actual_string` with curly
/// quotes (meaning `old_string` didn't match literally and we found a
/// normalized match), apply the file's curly-quote style to `new_string`
/// so the edit preserves typography.
///
/// Mirrors TS `preserveQuoteStyle`.
pub fn preserve_quote_style(old_string: &str, actual_old_string: &str, new_string: &str) -> String {
    if old_string == actual_old_string {
        return new_string.to_string();
    }

    let has_double = actual_old_string.contains(LEFT_DOUBLE_CURLY_QUOTE)
        || actual_old_string.contains(RIGHT_DOUBLE_CURLY_QUOTE);
    let has_single = actual_old_string.contains(LEFT_SINGLE_CURLY_QUOTE)
        || actual_old_string.contains(RIGHT_SINGLE_CURLY_QUOTE);

    if !has_double && !has_single {
        return new_string.to_string();
    }

    let mut result = new_string.to_string();
    if has_double {
        result = apply_curly_double_quotes(&result);
    }
    if has_single {
        result = apply_curly_single_quotes(&result);
    }
    result
}

/// Strip trailing whitespace from each line, preserving line endings.
/// Mirrors TS `stripTrailingWhitespace`.
pub fn strip_trailing_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for line_with_ending in split_preserving_endings(s) {
        let (content, ending) = split_line_ending(line_with_ending);
        out.push_str(content.trim_end());
        out.push_str(ending);
    }
    out
}

fn split_preserving_endings(s: &str) -> Vec<&str> {
    // Split on \r\n, \n, \r while keeping the separator attached to the
    // preceding content segment. We rebuild in the outer loop.
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut seg_start = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\r' {
            if i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                out.push(&s[seg_start..i + 2]);
                i += 2;
                seg_start = i;
                continue;
            }
            out.push(&s[seg_start..i + 1]);
            i += 1;
            seg_start = i;
            continue;
        }
        if b == b'\n' {
            out.push(&s[seg_start..i + 1]);
            i += 1;
            seg_start = i;
            continue;
        }
        i += 1;
    }
    if seg_start < bytes.len() {
        out.push(&s[seg_start..]);
    }
    out
}

fn split_line_ending(segment: &str) -> (&str, &str) {
    if let Some(s) = segment.strip_suffix("\r\n") {
        (s, "\r\n")
    } else if let Some(s) = segment.strip_suffix('\n') {
        (s, "\n")
    } else if let Some(s) = segment.strip_suffix('\r') {
        (s, "\r")
    } else {
        (segment, "")
    }
}

fn is_opening_context(chars: &[char], index: usize) -> bool {
    if index == 0 {
        return true;
    }
    matches!(
        chars[index - 1],
        ' ' | '\t' | '\n' | '\r' | '(' | '[' | '{' | '\u{2014}' | '\u{2013}'
    )
}

fn apply_curly_double_quotes(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    for i in 0..chars.len() {
        if chars[i] == '"' {
            if is_opening_context(&chars, i) {
                out.push(LEFT_DOUBLE_CURLY_QUOTE);
            } else {
                out.push(RIGHT_DOUBLE_CURLY_QUOTE);
            }
        } else {
            out.push(chars[i]);
        }
    }
    out
}

fn apply_curly_single_quotes(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    for i in 0..chars.len() {
        if chars[i] == '\'' {
            // Apostrophe in a contraction: letter-apostrophe-letter.
            let prev = if i > 0 { Some(chars[i - 1]) } else { None };
            let next = chars.get(i + 1).copied();
            let prev_is_letter = prev.is_some_and(|c| c.is_alphabetic());
            let next_is_letter = next.is_some_and(|c| c.is_alphabetic());
            if prev_is_letter && next_is_letter {
                out.push(RIGHT_SINGLE_CURLY_QUOTE);
            } else if is_opening_context(&chars, i) {
                out.push(LEFT_SINGLE_CURLY_QUOTE);
            } else {
                out.push(RIGHT_SINGLE_CURLY_QUOTE);
            }
        } else {
            out.push(chars[i]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_collapses_both_variants() {
        let input = "\u{201C}hello\u{201D} \u{2018}world\u{2019}";
        assert_eq!(normalize_quotes(input), "\"hello\" 'world'");
    }

    #[test]
    fn find_exact_match_returns_identity() {
        let h = "foo bar baz";
        assert_eq!(find_actual_string(h, "bar"), Some("bar"));
    }

    #[test]
    fn find_curly_to_straight_match() {
        let h = "He said \u{201C}hi\u{201D} then left.";
        let s = "\"hi\"";
        // The found substring is the CURLY version from the haystack.
        assert_eq!(find_actual_string(h, s), Some("\u{201C}hi\u{201D}"));
    }

    #[test]
    fn no_match_returns_none() {
        assert!(find_actual_string("one two three", "four").is_none());
    }

    #[test]
    fn preserve_quote_style_noop_on_exact_match() {
        let r = preserve_quote_style("foo", "foo", "bar");
        assert_eq!(r, "bar");
    }

    #[test]
    fn preserve_quote_style_applies_curly_double() {
        let r = preserve_quote_style("\"hi\"", "\u{201C}hi\u{201D}", "\"bye\"");
        assert_eq!(r, "\u{201C}bye\u{201D}");
    }

    #[test]
    fn preserve_quote_style_contractions_get_right_single() {
        // Replacement contains an apostrophe-in-contraction → right single.
        let r = preserve_quote_style("'x'", "\u{2018}x\u{2019}", "don't");
        // "don't" — the apostrophe is between letters, so right-single.
        assert!(r.contains(RIGHT_SINGLE_CURLY_QUOTE));
    }

    #[test]
    fn strip_trailing_whitespace_keeps_line_endings() {
        let s = "a    \nb\t\nc";
        let r = strip_trailing_whitespace(s);
        assert_eq!(r, "a\nb\nc");
    }

    #[test]
    fn strip_trailing_whitespace_handles_crlf() {
        let s = "a   \r\nb\r\n";
        let r = strip_trailing_whitespace(s);
        assert_eq!(r, "a\r\nb\r\n");
    }
}
