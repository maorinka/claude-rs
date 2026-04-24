//! Strip system-injected XML blocks from text for display titles.
//!
//! Port of TS `src/utils/displayTags.ts`. UI title fallback paths
//! (/rewind, /resume, bridge session titles) pass message content
//! through these helpers so IDE context, slash-command markers,
//! hook output, and task notifications never surface as titles.
//!
//! The TS regex uses a backreference (`<(\1)>...</\1>`); Rust's
//! `regex` crate doesn't support backrefs, so we walk the string
//! manually. Matching semantics:
//! - Opening tag: `<` + lowercase ASCII letter + any `\w` or `-`
//!   chars + optional attribute run `\s[^>]*` + `>`. Names starting
//!   with uppercase or `!` are skipped so user prose mentioning
//!   JSX/HTML components ("fix the <Button> layout",
//!   "<!DOCTYPE html>") passes through.
//! - Matching close: `</tagname>` literal. Scanning is non-greedy —
//!   the nearest matching close wins, so adjacent blocks stay
//!   separate.
//! - Whole block (including a single trailing `\n`) is removed.
//! - Unpaired `<` characters are left intact (so `x < y` survives).

/// IDE-injected tags that `strip_ide_context_tags` targets. Kept
/// in sync with the TS `IDE_CONTEXT_TAGS_PATTERN`.
pub const IDE_CONTEXT_TAG_NAMES: &[&str] = &["ide_opened_file", "ide_selection"];

/// Strip every lowercase-named `<tag>...</tag>` block from `text`
/// and trim the result. If the stripped result is empty, returns
/// the original text unchanged — better to show something than
/// nothing for a UI title.
pub fn strip_display_tags(text: &str) -> String {
    let stripped = strip_blocks(text, None).trim().to_string();
    if stripped.is_empty() {
        text.to_string()
    } else {
        stripped
    }
}

/// Like [`strip_display_tags`] but returns an empty string when
/// the input was entirely tag blocks. Used by the log/bridge title
/// paths to detect command-only prompts (e.g. `/clear`) so they
/// fall through to the next title fallback.
pub fn strip_display_tags_allow_empty(text: &str) -> String {
    strip_blocks(text, None).trim().to_string()
}

/// Strip ONLY the IDE-injected tags (`ide_opened_file`,
/// `ide_selection`). Used by `text_for_resubmit` so UP-arrow
/// resubmit keeps user-typed content — including lowercase HTML
/// like `<code>foo</code>` — while dropping IDE noise.
pub fn strip_ide_context_tags(text: &str) -> String {
    strip_blocks(text, Some(IDE_CONTEXT_TAG_NAMES))
        .trim()
        .to_string()
}

/// Core walker. When `allow_list` is `Some`, only matching tag
/// names get stripped; other tag blocks are left intact (but
/// scanned past, not re-entered).
fn strip_blocks(text: &str, allow_list: Option<&[&str]>) -> String {
    let bytes = text.as_bytes();
    let n = bytes.len();
    let mut out = String::with_capacity(text.len());
    let mut i = 0usize;

    while i < n {
        if bytes[i] != b'<' {
            // Copy byte — safe because we only ever split on ASCII
            // boundaries ('<', '>', tag-name ASCII chars).
            out.push(bytes[i] as char);
            i += 1;
            continue;
        }

        // Try to parse an opening tag at i.
        if let Some((name_start, name_end, open_end)) = parse_open_tag(bytes, i) {
            let name = std::str::from_utf8(&bytes[name_start..name_end]).unwrap_or("");
            let strip = match allow_list {
                None => true,
                Some(allowed) => allowed.contains(&name),
            };
            let close_seq = format!("</{name}>");
            if let Some(rel) = find_from(bytes, open_end, close_seq.as_bytes()) {
                let close_end = rel + close_seq.len();
                let block_end = if close_end < n && bytes[close_end] == b'\n' {
                    close_end + 1
                } else {
                    close_end
                };
                if strip {
                    i = block_end;
                    continue;
                } else {
                    // Leave tag-name-mismatched blocks intact.
                    out.push_str(std::str::from_utf8(&bytes[i..block_end]).unwrap_or(""));
                    i = block_end;
                    continue;
                }
            }
        }

        // Not a valid open tag — emit `<` literally.
        out.push('<');
        i += 1;
    }

    out
}

/// If `bytes[i..]` starts with an opening tag whose name begins
/// with a lowercase ASCII letter, return
/// `(name_start, name_end, index_after_closing_>)`.
fn parse_open_tag(bytes: &[u8], i: usize) -> Option<(usize, usize, usize)> {
    let n = bytes.len();
    if i + 1 >= n || bytes[i] != b'<' {
        return None;
    }
    let first = bytes[i + 1];
    if !first.is_ascii_lowercase() {
        return None;
    }
    let name_start = i + 1;
    let mut j = name_start + 1;
    while j < n {
        let c = bytes[j];
        if c.is_ascii_alphanumeric() || c == b'_' || c == b'-' {
            j += 1;
        } else {
            break;
        }
    }
    let name_end = j;
    // Either `>` immediately, or whitespace then anything non-`>`
    // until `>`.
    if j >= n {
        return None;
    }
    if bytes[j] == b'>' {
        return Some((name_start, name_end, j + 1));
    }
    if bytes[j].is_ascii_whitespace() {
        // Consume attributes: up to the next `>`.
        let mut k = j + 1;
        while k < n && bytes[k] != b'>' {
            k += 1;
        }
        if k < n && bytes[k] == b'>' {
            return Some((name_start, name_end, k + 1));
        }
    }
    None
}

fn find_from(haystack: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || from > haystack.len() {
        return None;
    }
    let n = haystack.len();
    let m = needle.len();
    if m > n.saturating_sub(from) {
        return None;
    }
    let mut i = from;
    while i + m <= n {
        if &haystack[i..i + m] == needle {
            return Some(i);
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_single_block() {
        let out = strip_display_tags("<hook>foo</hook>real title");
        assert_eq!(out, "real title");
    }

    #[test]
    fn strips_multiline_block_with_attributes() {
        let text = "<task-notification id=\"1\">\n  pending\n</task-notification>\ntitle here";
        let out = strip_display_tags(text);
        assert_eq!(out, "title here");
    }

    #[test]
    fn strips_multiple_adjacent_blocks() {
        let text = "<a>one</a><b>two</b> survivor";
        let out = strip_display_tags(text);
        assert_eq!(out, "survivor");
    }

    #[test]
    fn uppercase_or_bang_tags_are_preserved() {
        let text = "fix the <Button> layout";
        let out = strip_display_tags(text);
        assert_eq!(out, "fix the <Button> layout");
        let out2 = strip_display_tags("<!DOCTYPE html>hello");
        assert_eq!(out2, "<!DOCTYPE html>hello");
    }

    #[test]
    fn unpaired_lt_passes_through() {
        let out = strip_display_tags("when x < y then go");
        assert_eq!(out, "when x < y then go");
    }

    #[test]
    fn all_tags_falls_back_to_original() {
        let text = "<tag>only</tag>";
        let out = strip_display_tags(text);
        assert_eq!(out, text);
    }

    #[test]
    fn allow_empty_returns_empty_when_all_tags() {
        let text = "<tag>only</tag>";
        let out = strip_display_tags_allow_empty(text);
        assert_eq!(out, "");
    }

    #[test]
    fn nested_tag_names_are_not_matched_at_inner() {
        // The non-greedy "nearest match" semantics: `<a>` pairs with
        // the first `</a>`. TS regex had the same behaviour via
        // `[\s\S]*?` non-greedy body.
        let text = "<a>first</a>tail<a>second</a>";
        let out = strip_display_tags(text);
        assert_eq!(out, "tail");
    }

    #[test]
    fn trailing_newline_after_close_is_consumed() {
        let text = "<hook>x</hook>\nremaining";
        let out = strip_display_tags(text);
        assert_eq!(out, "remaining");
    }

    #[test]
    fn strip_ide_context_keeps_other_tags() {
        let text = "<ide_opened_file>foo</ide_opened_file><code>keep</code>after";
        let out = strip_ide_context_tags(text);
        assert_eq!(out, "<code>keep</code>after");
    }

    #[test]
    fn strip_ide_context_matches_all_ide_kinds() {
        let text = "<ide_opened_file>f</ide_opened_file><ide_selection>s</ide_selection>x";
        let out = strip_ide_context_tags(text);
        assert_eq!(out, "x");
    }

    #[test]
    fn hyphenated_tag_names_supported() {
        let out = strip_display_tags("<task-notification>x</task-notification>t");
        assert_eq!(out, "t");
    }

    #[test]
    fn dangling_open_tag_is_literal() {
        // No matching close — whole string survives.
        let out = strip_display_tags("<hook>no close");
        assert_eq!(out, "<hook>no close");
    }
}
