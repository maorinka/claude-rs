//! JSONL (JSON Lines) parsing — one JSON value per line,
//! malformed lines skipped.
//!
//! Port of the string path of TS `utils/json.ts:155-175`
//! (`parseJSONLString`). Buffer-based `parseJSONLBuffer` is
//! superfluous in Rust since we work with `&str` (already UTF-8
//! validated); the Bun-specific `parseJSONLBun` wrapper is
//! Node/Bun-specific and has no Rust equivalent.
//!
//! Semantics:
//! - Leading UTF-8 BOM is stripped before line iteration (TS
//!   calls `stripBOM`).
//! - Splits on `\n`. Trailing empty lines and whitespace-only
//!   lines are skipped.
//! - Each line is trimmed before parsing.
//! - Malformed lines are silently dropped. TS does not log or
//!   raise — the caller gets whatever survived.
//!
//! For a typed parse, wrap with `serde_json::from_str` at the
//! call site; this helper returns `Vec<serde_json::Value>` so
//! it stays generic over the payload shape.

use crate::string_utils::strip_bom;

/// Parse a JSONL string into a vector of `serde_json::Value`s.
/// Lines that fail to parse are silently skipped. Matches TS
/// `parseJSONLString` at `utils/json.ts:155-175`.
pub fn parse_jsonl(data: &str) -> Vec<serde_json::Value> {
    let stripped = strip_bom(data);
    let mut out = Vec::new();
    for line in stripped.split('\n') {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
            out.push(v);
        }
        // Malformed lines are dropped silently — matches TS
        // `catch {}` body at json.ts:171.
    }
    out
}

/// Generic variant: parse each JSONL line into `T` directly.
/// Useful when the caller knows the line shape and wants a
/// typed result without a second deserialization pass.
pub fn parse_jsonl_typed<T: serde::de::DeserializeOwned>(data: &str) -> Vec<T> {
    let stripped = strip_bom(data);
    let mut out = Vec::new();
    for line in stripped.split('\n') {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<T>(trimmed) {
            out.push(v);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_single_line() {
        let out = parse_jsonl(r#"{"k":1}"#);
        assert_eq!(out, vec![json!({"k": 1})]);
    }

    #[test]
    fn parses_multiple_lines() {
        let input = "{\"a\":1}\n{\"b\":2}\n{\"c\":3}";
        let out = parse_jsonl(input);
        assert_eq!(
            out,
            vec![json!({"a": 1}), json!({"b": 2}), json!({"c": 3})]
        );
    }

    #[test]
    fn skips_blank_and_whitespace_only_lines() {
        let input = "{\"a\":1}\n\n   \n{\"b\":2}\n\t  \t\n";
        let out = parse_jsonl(input);
        assert_eq!(out, vec![json!({"a": 1}), json!({"b": 2})]);
    }

    #[test]
    fn skips_malformed_lines_silently() {
        let input = "{\"a\":1}\nnot json at all\n{\"b\":2}";
        let out = parse_jsonl(input);
        assert_eq!(out, vec![json!({"a": 1}), json!({"b": 2})]);
    }

    #[test]
    fn strips_leading_bom() {
        let input = format!("{}{{\"a\":1}}", crate::string_utils::UTF8_BOM);
        let out = parse_jsonl(&input);
        assert_eq!(out, vec![json!({"a": 1})]);
    }

    #[test]
    fn empty_input_returns_empty_vec() {
        assert_eq!(parse_jsonl(""), Vec::<serde_json::Value>::new());
        assert_eq!(parse_jsonl("   \n\n\t"), Vec::<serde_json::Value>::new());
    }

    #[test]
    fn trailing_newline_is_not_an_empty_entry() {
        // `"{\"a\":1}\n"` → one entry, not two.
        let out = parse_jsonl("{\"a\":1}\n");
        assert_eq!(out, vec![json!({"a": 1})]);
    }

    #[test]
    fn accepts_scalar_and_array_lines() {
        // JSONL permits any JSON value per line, not just
        // objects. TS `JSON.parse` accepts all — so does ours.
        let input = "1\n\"hello\"\n[1,2,3]\nnull\ntrue";
        let out = parse_jsonl(input);
        assert_eq!(
            out,
            vec![
                json!(1),
                json!("hello"),
                json!([1, 2, 3]),
                json!(null),
                json!(true)
            ]
        );
    }

    #[test]
    fn typed_parse_deserializes_direct() {
        #[derive(serde::Deserialize, PartialEq, Debug)]
        struct Row {
            id: u32,
            name: String,
        }
        let input = concat!(
            r#"{"id":1,"name":"alice"}"#,
            "\n",
            r#"{"id":2,"name":"bob"}"#,
            "\n",
            r#"{"id":"bad"}"#, // type error → dropped
        );
        let rows: Vec<Row> = parse_jsonl_typed(input);
        assert_eq!(
            rows,
            vec![
                Row { id: 1, name: "alice".to_string() },
                Row { id: 2, name: "bob".to_string() }
            ]
        );
    }
}
