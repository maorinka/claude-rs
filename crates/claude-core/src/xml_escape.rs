//! XML / HTML entity escaping for safe interpolation.
//!
//! Port of TS `src/utils/xml.ts`. Used when interpolating untrusted
//! strings into XML element text or attribute values, e.g. building
//! the prompt-side `<bash-stdout>`, `<bash-stderr>`, or `<user_input>`
//! blocks.

/// Escape `&`, `<`, `>` for use inside element text content:
/// `<tag>{escape_xml(s)}</tag>`.
pub fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            other => out.push(other),
        }
    }
    out
}

/// Escape for interpolation into a quoted attribute value. Runs the
/// element-content escape first, then also replaces `"` and `'`.
pub fn escape_xml_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_xml_handles_three_chars() {
        assert_eq!(escape_xml("a & b"), "a &amp; b");
        assert_eq!(escape_xml("<tag>"), "&lt;tag&gt;");
    }

    #[test]
    fn escape_xml_amp_first_to_avoid_double_encoding() {
        // `&amp;` must be produced once — if `>` or `<` were
        // replaced before `&`, the inserted `&` from `&lt;` would be
        // re-escaped. Running `&` first is the standard fix and the
        // TS port's behaviour.
        assert_eq!(escape_xml("&<>"), "&amp;&lt;&gt;");
    }

    #[test]
    fn escape_xml_leaves_other_chars_alone() {
        assert_eq!(escape_xml("hello world 123"), "hello world 123");
        assert_eq!(escape_xml("\""), "\"");
        assert_eq!(escape_xml("'"), "'");
    }

    #[test]
    fn escape_xml_attr_also_handles_quotes() {
        assert_eq!(
            escape_xml_attr(r#"a "b" & <c>"#),
            "a &quot;b&quot; &amp; &lt;c&gt;"
        );
        assert_eq!(escape_xml_attr("a'b"), "a&apos;b");
    }

    #[test]
    fn escape_xml_empty_passes_through() {
        assert_eq!(escape_xml(""), "");
        assert_eq!(escape_xml_attr(""), "");
    }
}
