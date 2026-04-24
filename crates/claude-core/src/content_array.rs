//! Insert supplementary content blocks relative to tool_result blocks.
//!
//! Port of TS `src/utils/contentArray.ts`. The API layer uses this
//! to position cache-editing directives + other supplementary
//! blocks correctly within user messages.
//!
//! Placement rules (matching TS):
//! - When any `tool_result` block exists → insert immediately after
//!   the last one. If the insertion would leave the new block as
//!   the final element, append a text continuation block (`"."`)
//!   because some APIs reject messages that end with
//!   non-text content.
//! - When no `tool_result` is present → insert one slot *before*
//!   the last block (so the original tail stays final).
//! - Empty content array → insert at index 0.

use serde_json::{json, Value};

/// Mutate `content` in place: insert `block` per the positional
/// rules described on the module. Both `content` entries and `block`
/// are `serde_json::Value` so the helper works across the union of
/// block shapes (`text`, `image`, `tool_result`, etc.).
pub fn insert_block_after_tool_results(content: &mut Vec<Value>, block: Value) {
    let mut last_tool_result_index: Option<usize> = None;
    for (i, item) in content.iter().enumerate() {
        if is_tool_result(item) {
            last_tool_result_index = Some(i);
        }
    }

    match last_tool_result_index {
        Some(idx) => {
            let insert_pos = idx + 1;
            content.insert(insert_pos, block);
            if insert_pos == content.len() - 1 {
                content.push(json!({"type": "text", "text": "."}));
            }
        }
        None => {
            let insert_index = content.len().saturating_sub(1);
            content.insert(insert_index, block);
        }
    }
}

fn is_tool_result(item: &Value) -> bool {
    item.get("type")
        .and_then(Value::as_str)
        .map(|t| t == "tool_result")
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tr(id: &str) -> Value {
        json!({"type": "tool_result", "tool_use_id": id})
    }
    fn text(t: &str) -> Value {
        json!({"type": "text", "text": t})
    }

    #[test]
    fn inserts_after_last_tool_result() {
        let mut v = vec![
            text("before"),
            tr("a"),
            text("middle"),
            tr("b"),
            text("after"),
        ];
        insert_block_after_tool_results(&mut v, text("INSERTED"));
        assert_eq!(
            v,
            vec![
                text("before"),
                tr("a"),
                text("middle"),
                tr("b"),
                text("INSERTED"),
                text("after"),
            ]
        );
    }

    #[test]
    fn inserts_after_last_tool_result_and_appends_text_continuation() {
        let mut v = vec![text("before"), tr("a")];
        insert_block_after_tool_results(&mut v, text("INSERTED"));
        // inserted at index 2 (len 3), becomes final, so "." appended.
        assert_eq!(
            v,
            vec![text("before"), tr("a"), text("INSERTED"), text(".")]
        );
    }

    #[test]
    fn no_tool_results_inserts_before_last_block() {
        let mut v = vec![text("one"), text("two"), text("three")];
        insert_block_after_tool_results(&mut v, text("X"));
        assert_eq!(v, vec![text("one"), text("two"), text("X"), text("three")]);
    }

    #[test]
    fn no_tool_results_empty_array_inserts_at_zero() {
        let mut v: Vec<Value> = vec![];
        insert_block_after_tool_results(&mut v, text("X"));
        assert_eq!(v, vec![text("X")]);
    }

    #[test]
    fn no_tool_results_single_block() {
        let mut v = vec![text("only")];
        insert_block_after_tool_results(&mut v, text("X"));
        assert_eq!(v, vec![text("X"), text("only")]);
    }

    #[test]
    fn non_object_items_ignored() {
        // Non-object entries should not match tool_result detection.
        let mut v = vec![json!("raw string"), text("one")];
        insert_block_after_tool_results(&mut v, text("X"));
        assert_eq!(v, vec![json!("raw string"), text("X"), text("one")]);
    }
}
