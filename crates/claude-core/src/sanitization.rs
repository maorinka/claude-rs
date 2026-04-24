//! Unicode sanitisation for hidden-character attack mitigation.
//!
//! Port of TS `utils/sanitization.ts:1-91`.
//!
//! Defends against ASCII-smuggling / hidden-prompt-injection via invisible
//! Unicode (Tag characters, format controls, private-use areas, noncharacters).
//! See HackerOne #3086545 and the TS file's prose for background.
//!
//! Two entry points mirror the TS exports:
//! - [`partially_sanitize_unicode`] — string-only, the inner loop.
//! - [`recursively_sanitize_unicode`] — walks a `serde_json::Value`, which is
//!   Rust's natural analog to TS `unknown` for JSON payloads (the actual TS
//!   call sites all pass MCP tool-call inputs / outputs).

use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::{Map, Value};
use thiserror::Error;
use unicode_normalization::UnicodeNormalization;

/// TS uses a hard cap of 10 iterations (commands.ts:29). Matching verbatim.
const MAX_ITERATIONS: usize = 10;

/// Primary strip — Unicode general-category classes Cf (format),
/// Co (private-use), Cn (unassigned/noncharacter). The `regex` crate's
/// default `unicode-gencat` feature supplies these property escapes.
static DANGEROUS_CATEGORIES: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"[\p{Cf}\p{Co}\p{Cn}]").unwrap());

/// Fallback explicit ranges — TS keeps these alongside the property-class
/// strip because some engines (old Safari, shimmed V8) don't fully support
/// `\p{…}` for property classes. Rust's regex engine does, but preserving
/// the redundant pass keeps byte-for-byte parity and guards against any
/// future property-table drift in the regex crate's embedded tables.
static EXPLICIT_RANGES: Lazy<Regex> = Lazy::new(|| {
    Regex::new(concat!(
        "[",
        r"\u{200B}-\u{200F}", // Zero-width spaces, LTR/RTL marks
        r"\u{202A}-\u{202E}", // Directional formatting characters
        r"\u{2066}-\u{2069}", // Directional isolates
        r"\u{FEFF}",          // Byte order mark
        r"\u{E000}-\u{F8FF}", // BMP private-use area
        "]",
    ))
    .unwrap()
});

#[derive(Debug, Error)]
pub enum SanitizationError {
    /// TS throws a plain `Error` with the first 100 chars of the input (TS
    /// sanitization.ts:58-62). Rust surfaces it as a typed error so callers
    /// can match it without string parsing.
    #[error(
        "Unicode sanitization reached maximum iterations ({MAX_ITERATIONS}) for input: {preview}"
    )]
    MaxIterationsExceeded {
        /// First 100 chars of the original input — matches TS `prompt.slice(0, 100)`.
        /// Uses `chars().take(100)` to avoid slicing a UTF-8 boundary.
        preview: String,
    },
}

/// Strip hidden characters and apply NFKC, iterating until fixed-point.
///
/// Returns an error if 10 passes still change the string — means the input
/// is an adversarially-nested sequence that keeps re-inflating under NFKC.
pub fn partially_sanitize_unicode(prompt: &str) -> Result<String, SanitizationError> {
    let mut current = prompt.to_owned();
    let mut previous = String::new();
    let mut iterations = 0usize;

    while current != previous && iterations < MAX_ITERATIONS {
        previous = current.clone();

        // NFKC first — collapses composed sequences so the category strip
        // sees canonical forms.
        current = current.nfkc().collect::<String>();

        current = DANGEROUS_CATEGORIES.replace_all(&current, "").into_owned();
        current = EXPLICIT_RANGES.replace_all(&current, "").into_owned();

        iterations += 1;
    }

    if iterations >= MAX_ITERATIONS && current != previous {
        return Err(SanitizationError::MaxIterationsExceeded {
            preview: prompt.chars().take(100).collect(),
        });
    }

    Ok(current)
}

/// Recursively walk a JSON-shaped value, sanitising every string it contains —
/// including object keys, matching TS `recursivelySanitizeUnicode`
/// (sanitization.ts:81-85 walks `Object.entries` and rewrites keys).
///
/// Non-string leaves (numbers, bools, null) pass through unchanged.
pub fn recursively_sanitize_unicode(value: Value) -> Result<Value, SanitizationError> {
    match value {
        Value::String(s) => Ok(Value::String(partially_sanitize_unicode(&s)?)),
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(recursively_sanitize_unicode(item)?);
            }
            Ok(Value::Array(out))
        }
        Value::Object(map) => {
            let mut out = Map::with_capacity(map.len());
            for (k, v) in map {
                let clean_key = partially_sanitize_unicode(&k)?;
                out.insert(clean_key, recursively_sanitize_unicode(v)?);
            }
            Ok(Value::Object(out))
        }
        other => Ok(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ascii_passthrough() {
        assert_eq!(
            partially_sanitize_unicode("hello world").unwrap(),
            "hello world"
        );
    }

    #[test]
    fn strips_zero_width_spaces() {
        // U+200B, U+200C, U+200D — all in the \u{200B}-\u{200F} range.
        let input = "safe\u{200B}content\u{200C}here\u{200D}";
        assert_eq!(
            partially_sanitize_unicode(input).unwrap(),
            "safecontenthere"
        );
    }

    #[test]
    fn strips_bom() {
        assert_eq!(
            partially_sanitize_unicode("\u{FEFF}hello").unwrap(),
            "hello"
        );
    }

    #[test]
    fn strips_ltr_rtl_marks() {
        // U+200E LEFT-TO-RIGHT MARK, U+200F RIGHT-TO-LEFT MARK.
        let input = "a\u{200E}b\u{200F}c";
        assert_eq!(partially_sanitize_unicode(input).unwrap(), "abc");
    }

    #[test]
    fn strips_directional_formatting() {
        // U+202A..U+202E — LRE, RLE, PDF, LRO, RLO.
        for cp in 0x202Au32..=0x202E {
            let ch = char::from_u32(cp).unwrap();
            let input = format!("x{ch}y");
            assert_eq!(
                partially_sanitize_unicode(&input).unwrap(),
                "xy",
                "failed at U+{cp:04X}"
            );
        }
    }

    #[test]
    fn strips_directional_isolates() {
        // U+2066..U+2069 — LRI, RLI, FSI, PDI.
        for cp in 0x2066u32..=0x2069 {
            let ch = char::from_u32(cp).unwrap();
            let input = format!("x{ch}y");
            assert_eq!(
                partially_sanitize_unicode(&input).unwrap(),
                "xy",
                "failed at U+{cp:04X}"
            );
        }
    }

    #[test]
    fn strips_private_use_bmp() {
        let input = "a\u{E000}b\u{F8FF}c";
        assert_eq!(partially_sanitize_unicode(input).unwrap(), "abc");
    }

    #[test]
    fn strips_format_category() {
        // U+E0001 LANGUAGE TAG (Cf) — the core of the "Unicode Tag" attack.
        // Confirming a supplementary-plane Cf codepoint is caught by the
        // property-class strip (explicit ranges only cover BMP).
        let input = "cmd\u{E0001}extra";
        assert_eq!(partially_sanitize_unicode(input).unwrap(), "cmdextra");
    }

    #[test]
    fn nfkc_normalises_before_strip() {
        // Compatibility ligature — NFKC decomposes `ﬁ` (U+FB01) into "fi".
        assert_eq!(partially_sanitize_unicode("a\u{FB01}b").unwrap(), "afib");
    }

    #[test]
    fn idempotent_on_sanitised_input() {
        let clean = partially_sanitize_unicode("hello\u{200B}world").unwrap();
        let twice = partially_sanitize_unicode(&clean).unwrap();
        assert_eq!(clean, twice);
    }

    #[test]
    fn preserves_emoji_and_non_latin() {
        // Emoji (So), CJK (Lo), and combining marks (Mn) must pass through —
        // only Cf / Co / Cn are stripped.
        let input = "héllo 世界 🎉";
        assert_eq!(partially_sanitize_unicode(input).unwrap(), input);
    }

    #[test]
    fn recursive_sanitises_strings() {
        let v = json!("hi\u{200B}there");
        assert_eq!(recursively_sanitize_unicode(v).unwrap(), json!("hithere"));
    }

    #[test]
    fn recursive_walks_arrays() {
        let v = json!(["a\u{200B}b", "c\u{FEFF}d"]);
        assert_eq!(
            recursively_sanitize_unicode(v).unwrap(),
            json!(["ab", "cd"])
        );
    }

    #[test]
    fn recursive_walks_objects_and_keys() {
        // Both keys AND values must be sanitised — TS sanitization.ts:83-84
        // rewrites the key explicitly via the recursive call.
        let v = json!({"key\u{200B}1": "val\u{FEFF}ue"});
        assert_eq!(
            recursively_sanitize_unicode(v).unwrap(),
            json!({"key1": "value"})
        );
    }

    #[test]
    fn recursive_nested() {
        let v = json!({
            "outer": {
                "inner": ["a\u{200B}", {"k\u{FEFF}": "v\u{200C}"}]
            }
        });
        let expected = json!({
            "outer": {
                "inner": ["a", {"k": "v"}]
            }
        });
        assert_eq!(recursively_sanitize_unicode(v).unwrap(), expected);
    }

    #[test]
    fn recursive_leaves_primitives_alone() {
        let v = json!({"n": 42, "b": true, "nil": null, "f": 1.5});
        assert_eq!(recursively_sanitize_unicode(v.clone()).unwrap(), v);
    }
}
