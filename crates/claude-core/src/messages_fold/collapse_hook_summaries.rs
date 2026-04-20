//! Collapse consecutive same-label hook-summary messages.
//!
//! Port of TS `utils/collapseHookSummaries.ts:1-59`.
//!
//! **Reconstructed types disclaimer.** The TS file imports
//! `RenderableMessage` and `SystemStopHookSummaryMessage` from
//! `src/types/message.ts`, **which is missing from the leaked
//! source snapshot**. Rather than reverse-engineer the full TS
//! discriminated-union shape, this port works directly on
//! `serde_json::Value` — the same on-the-wire JSON shape the TS
//! messages cross the message stream as. Each helper documents the
//! exact fields it reads, so the contract with the missing type
//! source is explicit.
//!
//! Fields touched by this module
//! =============================
//! Read:
//! - `msg.type` (string) — discriminator, `"system"`
//! - `msg.subtype` (string) — discriminator, `"stop_hook_summary"`
//! - `msg.hookLabel` (string) — group key; TS undefined-check gates
//!   a message out of collapsing
//! - `msg.hookCount` (number) — summed across group
//! - `msg.hookInfos` (array) — flat-mapped across group
//! - `msg.hookErrors` (array) — flat-mapped across group
//! - `msg.preventedContinuation` (bool) — ORed across group
//! - `msg.hasOutput` (bool) — ORed across group
//! - `msg.totalDurationMs` (number?) — max across group (parallel
//!   tool calls overlap, so max ≈ wall-clock)
//!
//! Written (when synthesising a merged message):
//! - The same 6 fields above, overriding the first message's values
//!   on a clone. Other fields (uuid, timestamp, type, subtype,
//!   hookLabel, stopReason, level, toolUseID, any future additions)
//!   pass through from the first message via JSON spread semantics.
//!
//! Provenance fields referenced by TS that this port does NOT read
//! =================================================================
//! None beyond the list above — the TS source touches only those
//! fields (verified by reading `collapseHookSummaries.ts:1-59`).

use serde_json::{Map, Value};

/// `msg.type === 'system' && msg.subtype === 'stop_hook_summary' &&
/// msg.hookLabel !== undefined`. TS `isLabeledHookSummary`.
fn is_labeled_hook_summary(msg: &Value) -> bool {
    let Some(obj) = msg.as_object() else {
        return false;
    };
    if obj.get("type").and_then(Value::as_str) != Some("system") {
        return false;
    }
    if obj.get("subtype").and_then(Value::as_str) != Some("stop_hook_summary") {
        return false;
    }
    // TS `!== undefined` admits null OR missing. Keep the exact
    // check: present AND not null.
    match obj.get("hookLabel") {
        None => false,
        Some(Value::Null) => false,
        Some(_) => true,
    }
}

fn hook_label(msg: &Value) -> Option<&str> {
    msg.as_object()?.get("hookLabel")?.as_str()
}

/// Collapse consecutive stop-hook-summary messages with the same
/// `hookLabel` into a single synthesised message. Happens when
/// parallel tool calls each emit their own hook summary. TS
/// `collapseHookSummaries`.
///
/// Messages that don't match `is_labeled_hook_summary` pass through
/// untouched. Groups of exactly 1 also pass through untouched.
pub fn collapse_hook_summaries(messages: &[Value]) -> Vec<Value> {
    let mut result: Vec<Value> = Vec::with_capacity(messages.len());
    let mut i = 0usize;

    while i < messages.len() {
        let msg = &messages[i];
        if !is_labeled_hook_summary(msg) {
            result.push(msg.clone());
            i += 1;
            continue;
        }

        let label = hook_label(msg).unwrap_or_default().to_owned();
        let mut group: Vec<&Value> = Vec::new();
        while i < messages.len() {
            let next = &messages[i];
            if !is_labeled_hook_summary(next) || hook_label(next) != Some(label.as_str()) {
                break;
            }
            group.push(next);
            i += 1;
        }

        if group.len() == 1 {
            result.push(group[0].clone());
        } else {
            result.push(merge_group(group));
        }
    }

    result
}

/// Build the synthesised merged message. Matches TS `{...msg,
/// hookCount: ..., hookInfos: ..., hookErrors: ...,
/// preventedContinuation: ..., hasOutput: ..., totalDurationMs: ...}`.
fn merge_group(group: Vec<&Value>) -> Value {
    let first = group[0].clone();
    let mut base: Map<String, Value> = match first {
        Value::Object(m) => m,
        // Defensive — `is_labeled_hook_summary` required an object,
        // so a non-object here is a logic bug upstream.
        other => {
            let mut m = Map::new();
            m.insert("_value".into(), other);
            m
        }
    };

    // Sum hookCount across group. TS: `group.reduce((sum, m) => sum +
    // m.hookCount, 0)` on a `number` field.
    let hook_count: u64 = group
        .iter()
        .map(|m| {
            m.get("hookCount")
                .and_then(Value::as_u64)
                .unwrap_or(0)
        })
        .sum();
    base.insert("hookCount".into(), Value::from(hook_count));

    // Concat hookInfos. TS flatMap of arrays.
    let hook_infos: Vec<Value> = group
        .iter()
        .filter_map(|m| m.get("hookInfos").and_then(Value::as_array).cloned())
        .flatten()
        .collect();
    base.insert("hookInfos".into(), Value::Array(hook_infos));

    // Concat hookErrors. Same pattern.
    let hook_errors: Vec<Value> = group
        .iter()
        .filter_map(|m| m.get("hookErrors").and_then(Value::as_array).cloned())
        .flatten()
        .collect();
    base.insert("hookErrors".into(), Value::Array(hook_errors));

    // OR-reduce preventedContinuation / hasOutput.
    let prevented = group
        .iter()
        .any(|m| m.get("preventedContinuation").and_then(Value::as_bool).unwrap_or(false));
    base.insert("preventedContinuation".into(), Value::from(prevented));

    let has_output = group
        .iter()
        .any(|m| m.get("hasOutput").and_then(Value::as_bool).unwrap_or(false));
    base.insert("hasOutput".into(), Value::from(has_output));

    // Max totalDurationMs across group. TS comment: "Parallel tool
    // calls' hooks overlap; max is closest to wall-clock."
    let total_duration_ms: u64 = group
        .iter()
        .map(|m| {
            m.get("totalDurationMs")
                .and_then(Value::as_u64)
                .unwrap_or(0)
        })
        .max()
        .unwrap_or(0);
    base.insert("totalDurationMs".into(), Value::from(total_duration_ms));

    Value::Object(base)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn summary(label: &str, hook_count: u64, infos: Vec<&str>) -> Value {
        json!({
            "type": "system",
            "subtype": "stop_hook_summary",
            "hookLabel": label,
            "hookCount": hook_count,
            "hookInfos": infos,
            "hookErrors": [],
            "preventedContinuation": false,
            "hasOutput": false,
            "totalDurationMs": 50,
            "uuid": "u1",
            "timestamp": "2026-01-01T00:00:00Z",
        })
    }

    #[test]
    fn empty_input_returns_empty() {
        let out = collapse_hook_summaries(&[]);
        assert!(out.is_empty());
    }

    #[test]
    fn non_system_messages_pass_through() {
        let input = vec![
            json!({ "type": "user", "uuid": "u1" }),
            json!({ "type": "assistant", "uuid": "a1" }),
        ];
        let out = collapse_hook_summaries(&input);
        assert_eq!(out, input);
    }

    #[test]
    fn system_without_hook_label_passes_through() {
        // Subtype matches but hookLabel missing → not eligible.
        let input = vec![json!({
            "type": "system",
            "subtype": "stop_hook_summary",
            // no hookLabel
        })];
        let out = collapse_hook_summaries(&input);
        assert_eq!(out, input);
    }

    #[test]
    fn system_with_null_hook_label_passes_through() {
        // TS `!== undefined` — null counts as "not labeled" here
        // because the TS source uses `msg.hookLabel !== undefined`.
        // A Value::Null in Rust would require `.is_some() && !is_null`,
        // which matches.
        let input = vec![json!({
            "type": "system",
            "subtype": "stop_hook_summary",
            "hookLabel": null,
        })];
        let out = collapse_hook_summaries(&input);
        assert_eq!(out, input);
    }

    #[test]
    fn single_labeled_summary_passes_through_unchanged() {
        let input = vec![summary("PostToolUse", 1, vec!["hook-a"])];
        let out = collapse_hook_summaries(&input);
        assert_eq!(out, input);
    }

    #[test]
    fn two_same_label_collapse_into_one() {
        let input = vec![
            summary("PostToolUse", 1, vec!["hook-a"]),
            summary("PostToolUse", 2, vec!["hook-b", "hook-c"]),
        ];
        let out = collapse_hook_summaries(&input);
        assert_eq!(out.len(), 1);
        let merged = out.into_iter().next().unwrap();
        let obj = merged.as_object().unwrap();
        // Sum.
        assert_eq!(obj["hookCount"].as_u64(), Some(3));
        // Flat-map.
        let infos = obj["hookInfos"].as_array().unwrap();
        assert_eq!(infos.len(), 3);
        // First message's base fields preserved (uuid, timestamp etc).
        assert_eq!(obj["uuid"].as_str(), Some("u1"));
        assert_eq!(obj["hookLabel"].as_str(), Some("PostToolUse"));
    }

    #[test]
    fn different_labels_do_not_merge() {
        let input = vec![
            summary("PostToolUse", 1, vec!["a"]),
            summary("Stop", 1, vec!["b"]),
        ];
        let out = collapse_hook_summaries(&input);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn collapse_interrupted_by_other_message() {
        // Two Post → user msg → two Post should yield Post-group-of-2,
        // user, Post-group-of-2.
        let input = vec![
            summary("PostToolUse", 1, vec!["a"]),
            summary("PostToolUse", 1, vec!["b"]),
            json!({ "type": "user", "uuid": "u1" }),
            summary("PostToolUse", 1, vec!["c"]),
            summary("PostToolUse", 1, vec!["d"]),
        ];
        let out = collapse_hook_summaries(&input);
        assert_eq!(out.len(), 3);
        assert_eq!(out[1]["type"].as_str(), Some("user"));
        assert_eq!(out[0]["hookCount"].as_u64(), Some(2));
        assert_eq!(out[2]["hookCount"].as_u64(), Some(2));
    }

    #[test]
    fn prevented_continuation_ors() {
        let mut a = summary("Stop", 1, vec!["a"]);
        let mut b = summary("Stop", 1, vec!["b"]);
        a.as_object_mut().unwrap().insert("preventedContinuation".into(), Value::from(false));
        b.as_object_mut().unwrap().insert("preventedContinuation".into(), Value::from(true));

        let out = collapse_hook_summaries(&[a, b]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["preventedContinuation"].as_bool(), Some(true));
    }

    #[test]
    fn has_output_ors() {
        let mut a = summary("PostToolUse", 1, vec!["a"]);
        let mut b = summary("PostToolUse", 1, vec!["b"]);
        a.as_object_mut().unwrap().insert("hasOutput".into(), Value::from(true));
        b.as_object_mut().unwrap().insert("hasOutput".into(), Value::from(false));

        let out = collapse_hook_summaries(&[a, b]);
        assert_eq!(out[0]["hasOutput"].as_bool(), Some(true));
    }

    #[test]
    fn total_duration_takes_max_not_sum() {
        // TS comment says max is closer to wall-clock for parallel hooks.
        let mut a = summary("PostToolUse", 1, vec!["a"]);
        let mut b = summary("PostToolUse", 1, vec!["b"]);
        a.as_object_mut().unwrap().insert("totalDurationMs".into(), Value::from(100u64));
        b.as_object_mut().unwrap().insert("totalDurationMs".into(), Value::from(150u64));

        let out = collapse_hook_summaries(&[a, b]);
        assert_eq!(out[0]["totalDurationMs"].as_u64(), Some(150));
    }

    #[test]
    fn missing_optional_fields_default_to_zero_or_empty() {
        let a = json!({
            "type": "system",
            "subtype": "stop_hook_summary",
            "hookLabel": "X",
            "uuid": "u1",
        });
        let b = json!({
            "type": "system",
            "subtype": "stop_hook_summary",
            "hookLabel": "X",
            "uuid": "u2",
        });
        let out = collapse_hook_summaries(&[a, b]);
        assert_eq!(out.len(), 1);
        let obj = out[0].as_object().unwrap();
        assert_eq!(obj["hookCount"].as_u64(), Some(0));
        assert_eq!(obj["hookInfos"].as_array().unwrap().len(), 0);
        assert_eq!(obj["hookErrors"].as_array().unwrap().len(), 0);
        assert_eq!(obj["preventedContinuation"].as_bool(), Some(false));
        assert_eq!(obj["hasOutput"].as_bool(), Some(false));
        assert_eq!(obj["totalDurationMs"].as_u64(), Some(0));
    }

    #[test]
    fn first_message_base_fields_survive_merge() {
        // The TS spread `{...msg, ...overrides}` keeps base fields
        // from the first message. Pin: uuid, timestamp, level (if
        // present), stopReason (if present) all come from msg[0].
        let a = json!({
            "type": "system",
            "subtype": "stop_hook_summary",
            "hookLabel": "PostToolUse",
            "uuid": "first-uuid",
            "timestamp": "2026-04-20T00:00:00Z",
            "level": "info",
            "stopReason": "success",
            "hookCount": 1,
            "hookInfos": [],
            "hookErrors": [],
            "preventedContinuation": false,
            "hasOutput": false,
            "totalDurationMs": 10,
        });
        let b = json!({
            "type": "system",
            "subtype": "stop_hook_summary",
            "hookLabel": "PostToolUse",
            "uuid": "second-uuid",
            "timestamp": "2026-04-20T00:00:01Z",
            "level": "warning",
            "stopReason": "escalated",
            "hookCount": 1,
            "hookInfos": [],
            "hookErrors": [],
            "preventedContinuation": false,
            "hasOutput": false,
            "totalDurationMs": 20,
        });
        let out = collapse_hook_summaries(&[a, b]);
        let obj = out[0].as_object().unwrap();
        assert_eq!(obj["uuid"].as_str(), Some("first-uuid"));
        assert_eq!(obj["timestamp"].as_str(), Some("2026-04-20T00:00:00Z"));
        assert_eq!(obj["level"].as_str(), Some("info"));
        assert_eq!(obj["stopReason"].as_str(), Some("success"));
    }
}
