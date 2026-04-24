//! OSC 8 terminal hyperlinks.
//!
//! Port of TS `src/utils/hyperlink.ts`. Wraps a URL (and optional
//! display text) in the OSC 8 escape sequence so terminals that
//! support it render a clickable link. Falls back to the plain URL
//! when hyperlinks aren't supported.
//!
//! Colour rendering is left to callers — the TS file wrapped the
//! display text in `chalk.blue`, but that depends on the terminal
//! colour layer which isn't ported yet. This module returns raw
//! OSC 8 sequences around the text as-is.

pub const OSC8_START: &str = "\x1b]8;;";
pub const OSC8_END: &str = "\x07";

/// Build a clickable OSC 8 hyperlink. When `supports_hyperlinks`
/// is false, returns `url` unchanged. When `content` is `None`
/// the URL itself is used as the visible text.
pub fn create_hyperlink(url: &str, content: Option<&str>, supports_hyperlinks: bool) -> String {
    if !supports_hyperlinks {
        return url.to_string();
    }
    let text = content.unwrap_or(url);
    format!("{OSC8_START}{url}{OSC8_END}{text}{OSC8_START}{OSC8_END}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_url_when_unsupported() {
        assert_eq!(create_hyperlink("https://x", None, false), "https://x");
        assert_eq!(
            create_hyperlink("https://x", Some("hey"), false),
            "https://x"
        );
    }

    #[test]
    fn wraps_in_osc8_when_supported() {
        let out = create_hyperlink("https://x", Some("click"), true);
        let want = format!("{OSC8_START}https://x{OSC8_END}click{OSC8_START}{OSC8_END}");
        assert_eq!(out, want);
    }

    #[test]
    fn uses_url_as_text_when_content_missing() {
        let out = create_hyperlink("https://x", None, true);
        let want = format!("{OSC8_START}https://x{OSC8_END}https://x{OSC8_START}{OSC8_END}");
        assert_eq!(out, want);
    }

    #[test]
    fn constants_match_spec() {
        assert_eq!(OSC8_START, "\x1b]8;;");
        assert_eq!(OSC8_END, "\x07");
    }
}
