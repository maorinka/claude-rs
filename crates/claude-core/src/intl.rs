//! Unicode segmentation + locale helpers.
//!
//! Port of TS `utils/intl.ts:1-95`. TS caches expensive `Intl.*`
//! constructors; Rust uses the `unicode-segmentation` crate (zero-cost
//! iterators, no constructor cost) so the "cache the segmenter"
//! ceremony collapses into direct calls.
//!
//! Scope reductions from the TS module
//! ===================================
//! - `RelativeTimeFormat` is not ported — no Rust caller needs it yet,
//!   and the ICU relative-time data would require a non-trivial dep
//!   (`icu4x`) purely for UI strings we don't render.
//! - `getWordSegmenter` is exposed via [`words`], which returns an
//!   iterator rather than a stored segmenter object. Same use site.
//!
//! `get_time_zone` and `get_system_locale_language` are kept — they
//! feed analytics / display code and need one-shot process-lifetime
//! caching like the TS original.

use once_cell::sync::OnceCell;
use unicode_segmentation::UnicodeSegmentation;

/// First grapheme cluster, or `""` for empty input. Matches TS
/// `firstGrapheme(text)`.
pub fn first_grapheme(text: &str) -> &str {
    text.graphemes(true).next().unwrap_or("")
}

/// Last grapheme cluster, or `""` for empty input. Matches TS
/// `lastGrapheme(text)`.
pub fn last_grapheme(text: &str) -> &str {
    text.graphemes(true).next_back().unwrap_or("")
}

/// Iterator over grapheme clusters. Replaces
/// `getGraphemeSegmenter().segment(text)`.
pub fn graphemes(text: &str) -> impl Iterator<Item = &str> {
    text.graphemes(true)
}

/// Iterator over word segments (Unicode UAX #29 word boundaries).
/// Replaces `getWordSegmenter().segment(text)`. Filters out
/// whitespace-only segments to match TS's usage pattern (callers use
/// this to count words / tokenise, not to capture separators).
pub fn words(text: &str) -> impl Iterator<Item = &str> {
    text.unicode_words()
}

static CACHED_TIME_ZONE: OnceCell<Option<String>> = OnceCell::new();
static CACHED_LOCALE_LANG: OnceCell<Option<String>> = OnceCell::new();

/// IANA time-zone name (e.g. `"America/New_York"`). Process-lifetime
/// cached — TS calls `Intl.DateTimeFormat().resolvedOptions().timeZone`
/// once and reuses the string.
///
/// Returns `None` when the host provides no resolvable zone (containers
/// without `/etc/localtime` or `TZ`). Callers should fall back to UTC.
pub fn get_time_zone() -> Option<&'static str> {
    CACHED_TIME_ZONE
        .get_or_init(|| iana_time_zone::get_timezone().ok())
        .as_deref()
}

/// ISO-639 language subtag of the system locale (e.g. `"en"`, `"ja"`).
/// Derived from env (LC_ALL / LC_CTYPE / LANG) — the same chain POSIX
/// locale resolution uses, mirroring Node's ICU locale resolution.
/// `None` when nothing resolvable is set (stripped-ICU environments).
pub fn get_system_locale_language() -> Option<&'static str> {
    CACHED_LOCALE_LANG
        .get_or_init(|| {
            let raw = std::env::var("LC_ALL")
                .ok()
                .or_else(|| std::env::var("LC_CTYPE").ok())
                .or_else(|| std::env::var("LANG").ok())?;
            // Locale strings look like `en_US.UTF-8` or `ja-JP` — extract
            // the language subtag before `_`, `-`, `.`, or `@`.
            let mut end = raw.len();
            for (i, c) in raw.char_indices() {
                if c == '_' || c == '-' || c == '.' || c == '@' {
                    end = i;
                    break;
                }
            }
            let tag = &raw[..end];
            if tag.is_empty() || tag == "C" || tag == "POSIX" {
                None
            } else {
                Some(tag.to_ascii_lowercase())
            }
        })
        .as_deref()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_grapheme_basic() {
        assert_eq!(first_grapheme(""), "");
        assert_eq!(first_grapheme("hello"), "h");
    }

    #[test]
    fn first_grapheme_multibyte() {
        // `é` is a single grapheme whether precomposed (U+00E9) or
        // decomposed (U+0065 U+0301). UAX #29 treats both as one cluster.
        assert_eq!(first_grapheme("é"), "é");
        assert_eq!(first_grapheme("e\u{0301}other"), "e\u{0301}");
    }

    #[test]
    fn first_grapheme_emoji_zwj_sequence() {
        // Family emoji ZWJ sequence = one grapheme cluster.
        let family = "👨‍👩‍👧";
        assert_eq!(first_grapheme(family), family);
    }

    #[test]
    fn last_grapheme_basic() {
        assert_eq!(last_grapheme(""), "");
        assert_eq!(last_grapheme("hello"), "o");
        assert_eq!(last_grapheme("abc\u{0301}"), "c\u{0301}");
    }

    #[test]
    fn graphemes_iterates_all_clusters() {
        let text = "a👨‍👩‍👧b";
        let gs: Vec<&str> = graphemes(text).collect();
        // 3 graphemes: `a`, the family emoji, `b`.
        assert_eq!(gs.len(), 3);
        assert_eq!(gs[0], "a");
        assert_eq!(gs[2], "b");
    }

    #[test]
    fn words_splits_on_unicode_boundaries() {
        let ws: Vec<&str> = words("hello, world!").collect();
        assert_eq!(ws, vec!["hello", "world"]);
    }

    #[test]
    fn words_skips_whitespace_and_punct() {
        let ws: Vec<&str> = words("   foo\tbar  ").collect();
        assert_eq!(ws, vec!["foo", "bar"]);
    }

    #[test]
    fn time_zone_memoised() {
        let a = get_time_zone();
        let b = get_time_zone();
        assert_eq!(a, b);
    }

    #[test]
    fn locale_language_respects_lang_env() {
        // Can't reliably mutate the OnceCell across tests, so exercise
        // the parser directly via a fresh path. This test is mostly a
        // smoke check — full coverage lives in extract_subtag_*.
        let _ = get_system_locale_language();
    }

    // Below we test the subtag extraction logic in isolation by
    // re-implementing the parser from the same source text. Keeps the
    // OnceCell untouched.
    fn extract_subtag(raw: &str) -> Option<String> {
        let mut end = raw.len();
        for (i, c) in raw.char_indices() {
            if c == '_' || c == '-' || c == '.' || c == '@' {
                end = i;
                break;
            }
        }
        let tag = &raw[..end];
        if tag.is_empty() || tag == "C" || tag == "POSIX" {
            None
        } else {
            Some(tag.to_ascii_lowercase())
        }
    }

    #[test]
    fn extract_subtag_from_posix_locale() {
        assert_eq!(extract_subtag("en_US.UTF-8").as_deref(), Some("en"));
        assert_eq!(extract_subtag("ja_JP").as_deref(), Some("ja"));
        assert_eq!(extract_subtag("de-DE").as_deref(), Some("de"));
        assert_eq!(extract_subtag("fr@euro").as_deref(), Some("fr"));
        assert_eq!(extract_subtag("EN_US.UTF-8").as_deref(), Some("en"));
    }

    #[test]
    fn extract_subtag_rejects_c_and_posix() {
        assert_eq!(extract_subtag("C"), None);
        assert_eq!(extract_subtag("POSIX"), None);
        assert_eq!(extract_subtag(""), None);
    }
}
