//! MCP tool-result transform utilities.
//!
//! Gap-fill ticket **G12a** in the MCP client plan. Ports the pure
//! structural helpers from `src/services/mcp/client.ts:2644-2718`:
//!
//! - `infer_compact_schema(value, depth)` — jq-friendly type
//!   signature for a JSON value; used in `structuredContent`
//!   formatting and large-output persistence descriptions.
//! - `content_contains_images(content)` — predicate used to decide
//!   whether to truncate-vs-persist large tool results. Persisting
//!   images as JSON defeats the image compression pipeline and
//!   makes them non-viewable, so images short-circuit to
//!   truncation.
//!
//! The full `transformResultContent` / `transformMCPResult` /
//! `processMCPResult` pipeline (TS `client.ts:2478-2799`) is the
//! larger G12b/c work — it needs ported image-resize (Sharp-
//! equivalent), binary-persistence, and token-counting
//! infrastructure. Those land in follow-up tickets.

/// Build a compact, jq-friendly type signature for an arbitrary JSON
/// value. Matches TS `inferCompactSchema` at `client.ts:2644-2660`:
///
/// - `null` → `"null"`
/// - Arrays: `[{T}]` (first-element shape) or `"[]"` for empty.
/// - Objects: `{k1: T1, k2: T2, ...}` with at most 10 entries; a
///   trailing `", ..."` marks truncated object keys; depth `0`
///   collapses to `"{...}"`.
/// - Scalars: the JavaScript `typeof` word — `"string"`,
///   `"number"`, `"boolean"`. Rust JSON has no undefined /
///   function / symbol, so those aren't emitted.
///
/// The output is NOT valid JSON — it's a schema sketch for humans
/// (and our own prompt templates) to scan quickly.
pub fn infer_compact_schema(value: &serde_json::Value, depth: i32) -> String {
    use serde_json::Value;

    match value {
        Value::Null => "null".to_string(),
        Value::Array(arr) => {
            if arr.is_empty() {
                "[]".to_string()
            } else {
                format!("[{}]", infer_compact_schema(&arr[0], depth - 1))
            }
        }
        Value::Object(map) => {
            if depth <= 0 {
                return "{...}".to_string();
            }
            let total = map.len();
            let props: Vec<String> = map
                .iter()
                .take(10)
                .map(|(k, v)| format!("{}: {}", k, infer_compact_schema(v, depth - 1)))
                .collect();
            let suffix = if total > 10 { ", ..." } else { "" };
            format!("{{{}{}}}", props.join(", "), suffix)
        }
        Value::Bool(_) => "boolean".to_string(),
        // TS `typeof 42 === 'number'` regardless of integer vs float.
        Value::Number(_) => "number".to_string(),
        Value::String(_) => "string".to_string(),
    }
}

/// Same default depth TS uses (`inferCompactSchema(value, depth = 2)`
/// at `client.ts:2644`).
pub const DEFAULT_SCHEMA_DEPTH: i32 = 2;

/// Convenience: shape-sketch with the default depth.
pub fn infer_compact_schema_default(value: &serde_json::Value) -> String {
    infer_compact_schema(value, DEFAULT_SCHEMA_DEPTH)
}

/// Does any block in the MCP tool-result content array carry an
/// image payload? Used by the large-output dispatcher to decide
/// truncate-vs-persist: persisting an image as JSON would defeat
/// the image-compression pipeline and make it non-viewable, so
/// image-containing results short-circuit to the truncation
/// branch. Matches TS `contentContainsImages` at
/// `client.ts:2713-2718`.
///
/// Accepts the Rust `McpToolResultContent` shape; a block counts as
/// an image when its `content_type` field equals `"image"` —
/// downstream variants like `resource` (with embedded base64) are
/// detected after `transform_result_content` expands them, not
/// here.
///
/// TS `contentContainsImages` is typed `MCPToolResult` which is
/// `string | ContentBlock[]` and returns `false` for the string
/// case. Rust's `McpToolResult.content` is always an array so the
/// string branch has no representation today; if G12b/c introduces
/// a union where post-transform content can be a string,
/// add a wrapper that short-circuits to `false` for strings.
pub fn content_contains_images(content: &[crate::mcp::types::McpToolResultContent]) -> bool {
    content.iter().any(|b| b.content_type == "image")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::types::McpToolResultContent;
    use serde_json::json;

    // ─── infer_compact_schema ────────────────────────────────────

    #[test]
    fn schema_scalars() {
        assert_eq!(infer_compact_schema(&json!(null), 2), "null");
        assert_eq!(infer_compact_schema(&json!(true), 2), "boolean");
        assert_eq!(infer_compact_schema(&json!(42), 2), "number");
        assert_eq!(infer_compact_schema(&json!(3.14), 2), "number");
        assert_eq!(infer_compact_schema(&json!("hi"), 2), "string");
    }

    #[test]
    fn schema_arrays_use_first_element_shape() {
        assert_eq!(infer_compact_schema(&json!([]), 2), "[]");
        assert_eq!(infer_compact_schema(&json!([1, 2, 3]), 2), "[number]");
        assert_eq!(
            infer_compact_schema(&json!(["a", "b"]), 2),
            "[string]"
        );
        // Nested array of objects.
        assert_eq!(
            infer_compact_schema(&json!([{"id": 1, "name": "x"}]), 3),
            "[{id: number, name: string}]"
        );
    }

    #[test]
    fn schema_objects_include_field_types() {
        let v = json!({"title": "x", "count": 3});
        let s = infer_compact_schema(&v, 2);
        // serde's Map preserves insertion order on the parse path.
        assert!(s.starts_with('{') && s.ends_with('}'));
        assert!(s.contains("title: string"));
        assert!(s.contains("count: number"));
    }

    #[test]
    fn schema_object_depth_zero_collapses() {
        // At depth 0, an object becomes "{...}" even if it has
        // fields — matches TS guard.
        assert_eq!(infer_compact_schema(&json!({"a": 1}), 0), "{...}");
        // Depth 1 shows fields but scalar recursion hits depth 0
        // (scalars ignore depth so they still print). Arrays of
        // objects at depth 1 recurse to depth 0 on the inner
        // object → "{...}".
        assert_eq!(
            infer_compact_schema(&json!([{"a": 1}]), 1),
            "[{...}]"
        );
    }

    #[test]
    fn schema_truncates_after_ten_object_keys() {
        let mut m = serde_json::Map::new();
        for i in 0..12 {
            m.insert(format!("k{:02}", i), json!(i));
        }
        let s = infer_compact_schema(&serde_json::Value::Object(m), 2);
        // 10 entries surface; the remaining two are marked by
        // ", ...".
        let commas = s.matches(',').count();
        // 10 entries = 9 separators + ", ..." marker = 10 commas.
        assert!(
            commas >= 10,
            "expected >= 10 commas in {}",
            s
        );
        assert!(s.ends_with(", ...}"));
    }

    #[test]
    fn schema_exactly_ten_keys_has_no_truncation_marker() {
        // Codex CR gap: 10 keys is the cut-off — exactly 10 must
        // NOT have ", ..." appended (TS `map > 10 ? ", ..." : ""`
        // strict-greater). Guards against a regression where the
        // boundary condition flips to `>=`.
        let mut m = serde_json::Map::new();
        for i in 0..10 {
            m.insert(format!("k{}", i), json!(i));
        }
        let s = infer_compact_schema(&serde_json::Value::Object(m), 2);
        assert!(
            !s.contains(", ...}"),
            "exactly-10 keys should not produce the '... ,' marker; got {}",
            s
        );
    }

    #[test]
    fn schema_large_integer_is_still_number() {
        // TS would report this as "number" even if it overflows
        // JS-safe integers; Rust serde_json parses it into
        // Value::Number regardless of width. Assert parity.
        let v = json!(12345678901234567890u64);
        assert_eq!(infer_compact_schema(&v, 2), "number");
    }

    #[test]
    fn schema_array_of_arrays_uses_first_element_only() {
        // First-element recursion: `[[{...}], ["x"]]` recurses
        // only on the first element `[{...}]` → `[[{...}]]`.
        // Guards against any accidental "survey all elements"
        // behaviour.
        let v = json!([[{"a": 1}], ["x"]]);
        let s = infer_compact_schema(&v, 3);
        assert!(
            s.starts_with("[[{") && s.ends_with("}]]"),
            "first-element recursion broken for array-of-arrays: {}",
            s
        );
    }

    #[test]
    fn schema_default_depth_matches_ts() {
        // TS default is 2 — verify the convenience fn picks it up.
        let v = json!({"outer": {"inner": "v"}});
        assert_eq!(
            infer_compact_schema_default(&v),
            infer_compact_schema(&v, 2)
        );
    }

    // ─── content_contains_images ─────────────────────────────────

    fn block(ty: &str) -> McpToolResultContent {
        McpToolResultContent {
            content_type: ty.to_string(),
            text: None,
            data: None,
            mime_type: None,
        }
    }

    #[test]
    fn contains_images_empty_is_false() {
        assert!(!content_contains_images(&[]));
    }

    #[test]
    fn contains_images_text_only_is_false() {
        let blocks = vec![block("text"), block("text")];
        assert!(!content_contains_images(&blocks));
    }

    #[test]
    fn contains_images_single_image_is_true() {
        let blocks = vec![block("text"), block("image"), block("text")];
        assert!(content_contains_images(&blocks));
    }

    #[test]
    fn contains_images_does_not_match_audio_or_resource() {
        // audio / resource blocks count as binary but NOT as images
        // for the truncate-vs-persist decision. TS
        // `contentContainsImages` only inspects `block.type ===
        // 'image'`.
        let blocks = vec![block("audio"), block("resource")];
        assert!(!content_contains_images(&blocks));
    }
}
