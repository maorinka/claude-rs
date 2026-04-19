//! String utilities ported from `src/utils/stringUtils.ts`.
//!
//! Small pure helpers used throughout TS. Scope of this port: the
//! pure + broadly-useful ones (escape_regex, capitalize, plural,
//! first_line_of, count_char_in, normalize_full_width_digits /
//! _space, safe_join_lines, truncate_to_lines). Skipped: the class-
//! based SafeStringAccumulator (TS uses it with Buffer which doesn't
//! exist in Rust) — callers that need accumulation can use String +
//! push_str directly.

/// UTF-8 byte-order mark (U+FEFF). PowerShell 5.x writes UTF-8
/// with BOM by default (`Out-File`, `Set-Content`). Without
/// stripping, a BOM-prefixed JSON file fails to parse with
/// "Unexpected token". Matches TS `utils/jsonRead.ts:12`.
pub const UTF8_BOM: &str = "\u{FEFF}";

/// Strip a leading UTF-8 BOM if present; otherwise return the
/// input unchanged. Pure, zero-allocation on the no-BOM path.
/// TS `stripBOM` at `utils/jsonRead.ts:14-16`.
pub fn strip_bom(content: &str) -> &str {
    content.strip_prefix(UTF8_BOM).unwrap_or(content)
}

/// Escape regex metacharacters so `str` is matched literally.
/// Matches TS `escapeRegExp` verbatim.
pub fn escape_regex(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(
            c,
            '.' | '*' | '+' | '?' | '^' | '$' | '{' | '}' | '(' | ')' | '|' | '[' | ']' | '\\'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Uppercase the first character. Unlike lodash `capitalize`, leaves
/// the rest of the string unchanged.
pub fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_ascii_uppercase().to_string() + chars.as_str(),
    }
}

/// Singular or plural form of a word based on count. `plural_word`
/// defaults to `word + "s"`.
pub fn plural<'a>(n: usize, word: &'a str, plural_word: Option<&'a str>) -> String {
    if n == 1 {
        word.to_string()
    } else {
        plural_word.map(str::to_string).unwrap_or_else(|| format!("{}s", word))
    }
}

/// First line of a string (up to but not including `\n`).
pub fn first_line_of(s: &str) -> &str {
    match s.find('\n') {
        Some(i) => &s[..i],
        None => s,
    }
}

/// Count occurrences of `needle` in `haystack`. Non-overlapping.
pub fn count_char_in(haystack: &str, needle: char) -> usize {
    haystack.chars().filter(|c| *c == needle).count()
}

/// Normalize full-width (zenkaku) digits `０-９` to half-width `0-9`.
/// Accepts input from Japanese/CJK IMEs.
pub fn normalize_full_width_digits(input: &str) -> String {
    input
        .chars()
        .map(|c| match c {
            '０'..='９' => (c as u32 - 0xFEE0) as u8 as char,
            other => other,
        })
        .collect()
}

/// Normalize full-width space `U+3000` to regular space `U+0020`.
pub fn normalize_full_width_space(input: &str) -> String {
    input.replace('\u{3000}', " ")
}

/// Max in-memory string size before callers should spill to disk.
pub const MAX_STRING_LENGTH: usize = 1 << 25; // 32 MiB

/// Join `lines` with `delimiter`, truncating the result once it
/// reaches `max_size` characters. Appends "... [truncated]" when cut.
pub fn safe_join_lines(lines: &[&str], delimiter: &str, max_size: usize) -> String {
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            out.push_str(delimiter);
        }
        if out.len() + line.len() > max_size {
            let room = max_size.saturating_sub(out.len());
            if room > 0 {
                let mut cut = room.min(line.len());
                while cut > 0 && !line.is_char_boundary(cut) {
                    cut -= 1;
                }
                out.push_str(&line[..cut]);
            }
            out.push_str("... [truncated]");
            break;
        }
        out.push_str(line);
    }
    out
}

/// Keep the first `max_lines` lines of `text`; drop the rest,
/// appending a "\n... [<n> more lines]" marker when cut.
pub fn truncate_to_lines(text: &str, max_lines: usize) -> String {
    let mut out = String::new();
    let mut taken = 0usize;
    let total_lines = text.lines().count();
    for line in text.lines().take(max_lines) {
        if taken > 0 {
            out.push('\n');
        }
        out.push_str(line);
        taken += 1;
    }
    if total_lines > max_lines {
        out.push_str(&format!(
            "\n... [{} more {}]",
            total_lines - max_lines,
            if total_lines - max_lines == 1 { "line" } else { "lines" }
        ));
    }
    out
}

// ── Token estimation ──────────────────────────────────────────────────────
//
// Port of the rough-estimate half of `src/services/tokenEstimation.ts`.
// The full service also calls Haiku via the API to get real counts
// when available — that path depends on the secondary_model trait +
// file-type-specific bytes-per-token heuristics we port here for the
// fallback. The Haiku caller belongs alongside tool_use_summary when
// we wire a real token-count helper on top of the secondary model.

/// Estimate token count from a character length. Matches TS
/// `roughTokenCountEstimation` default ratio (4).
pub fn rough_token_count_estimation(content: &str) -> usize {
    rough_token_count_estimation_with_ratio(content, 4)
}

pub fn rough_token_count_estimation_with_ratio(content: &str, bytes_per_token: usize) -> usize {
    if bytes_per_token == 0 {
        return content.len();
    }
    // TS uses Math.round — banker's round isn't strictly required; the
    // caller only reads trends. Use nearest-integer.
    (content.len() + bytes_per_token / 2) / bytes_per_token
}

/// Bytes-per-token for a file extension. JSON family is denser (more
/// single-char tokens) so it maps ~2 bytes/token rather than the
/// default 4.
pub fn bytes_per_token_for_file_type(file_extension: &str) -> usize {
    match file_extension {
        "json" | "jsonl" | "jsonc" => 2,
        _ => 4,
    }
}

/// Like `rough_token_count_estimation` but uses the file-type
/// heuristic when the extension is known.
pub fn rough_token_count_estimation_for_file_type(content: &str, file_extension: &str) -> usize {
    rough_token_count_estimation_with_ratio(
        content,
        bytes_per_token_for_file_type(file_extension),
    )
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_regex_works() {
        assert_eq!(escape_regex("foo.bar*"), "foo\\.bar\\*");
        assert_eq!(escape_regex("abc"), "abc");
    }

    #[test]
    fn strip_bom_removes_leading_feff() {
        // UTF-8 BOM is 0xEF 0xBB 0xBF (3 bytes) which encodes
        // U+FEFF as a single scalar. Constructed via the
        // escape so the source stays ASCII.
        let with_bom = format!("{}{{\"k\":1}}", UTF8_BOM);
        assert_eq!(strip_bom(&with_bom), "{\"k\":1}");
    }

    #[test]
    fn strip_bom_passes_through_when_absent() {
        assert_eq!(strip_bom("{\"k\":1}"), "{\"k\":1}");
        assert_eq!(strip_bom(""), "");
        // A BOM in the middle of the string is NOT stripped —
        // TS only checks `startsWith`.
        let mid = format!("hello {} world", UTF8_BOM);
        assert_eq!(strip_bom(&mid), mid);
    }

    #[test]
    fn strip_bom_returns_borrowed_slice() {
        // Zero-alloc path: pass-through returns a sub-slice of
        // the input. After `strip_bom(s)` the result's bytes
        // must live inside `s`.
        let s = String::from("plain");
        let out = strip_bom(&s);
        assert_eq!(out.as_ptr(), s.as_ptr());
    }

    #[test]
    fn capitalize_preserves_rest() {
        assert_eq!(capitalize("fooBar"), "FooBar");
        assert_eq!(capitalize("hello world"), "Hello world");
        assert_eq!(capitalize(""), "");
    }

    #[test]
    fn plural_picks_correct_form() {
        assert_eq!(plural(1, "file", None), "file");
        assert_eq!(plural(3, "file", None), "files");
        assert_eq!(plural(2, "entry", Some("entries")), "entries");
    }

    #[test]
    fn first_line_of_handles_missing_newline() {
        assert_eq!(first_line_of("just one line"), "just one line");
        assert_eq!(first_line_of("line1\nline2"), "line1");
    }

    #[test]
    fn count_char_matches() {
        assert_eq!(count_char_in("aaab", 'a'), 3);
        assert_eq!(count_char_in("abc", 'z'), 0);
    }

    #[test]
    fn full_width_digits_normalize() {
        assert_eq!(normalize_full_width_digits("０１２abc"), "012abc");
    }

    #[test]
    fn full_width_space_normalizes() {
        assert_eq!(
            normalize_full_width_space("foo\u{3000}bar"),
            "foo bar"
        );
    }

    #[test]
    fn safe_join_lines_truncates() {
        let lines = vec!["aaaa", "bbbb", "cccc"];
        let out = safe_join_lines(&lines, ",", 8);
        assert!(out.ends_with("[truncated]"));
    }

    #[test]
    fn safe_join_lines_fits() {
        let lines = vec!["a", "b", "c"];
        assert_eq!(safe_join_lines(&lines, ",", 100), "a,b,c");
    }

    #[test]
    fn truncate_to_lines_counts_remainder() {
        let text = "1\n2\n3\n4\n5";
        let out = truncate_to_lines(text, 2);
        assert_eq!(out, "1\n2\n... [3 more lines]");
    }

    #[test]
    fn truncate_to_lines_no_truncation_needed() {
        assert_eq!(truncate_to_lines("a\nb", 5), "a\nb");
    }

    #[test]
    fn rough_tokens_default_ratio() {
        assert_eq!(rough_token_count_estimation(""), 0);
        // "abcd" -> 4 bytes / 4 bpt = 1
        assert_eq!(rough_token_count_estimation("abcd"), 1);
        // "abcdefgh" -> 8/4 = 2
        assert_eq!(rough_token_count_estimation("abcdefgh"), 2);
    }

    #[test]
    fn bytes_per_token_json_denser() {
        assert_eq!(bytes_per_token_for_file_type("json"), 2);
        assert_eq!(bytes_per_token_for_file_type("jsonl"), 2);
        assert_eq!(bytes_per_token_for_file_type("jsonc"), 2);
        assert_eq!(bytes_per_token_for_file_type("ts"), 4);
    }

    #[test]
    fn rough_tokens_file_type_applies_denser_ratio() {
        let json = r#"{"a":1}"#;
        let default = rough_token_count_estimation(json);
        let as_json = rough_token_count_estimation_for_file_type(json, "json");
        assert!(as_json > default);
    }
}
