//! Group consecutive parallel-tool-use messages by (message_id, tool_name).
//!
//! Port of TS `utils/groupToolUses.ts:1-182`.
//!
//! **Reconstructed types disclaimer.** The TS file imports
//! `GroupedToolUseMessage`, `NormalizedAssistantMessage<BetaToolUseBlock>`,
//! `NormalizedMessage`, `NormalizedUserMessage`, `ProgressMessage`,
//! and `RenderableMessage` from `src/types/message.ts`, **which is
//! missing from the leaked source snapshot**. It also references
//! `Tool` (with a `renderGroupedToolUse` boolean flag) from
//! `src/Tool.ts`. This port works on `serde_json::Value` streams +
//! takes the set of grouping-enabled tool names as a parameter,
//! sidestepping both missing graphs.
//!
//! Fields touched
//! ==============
//! Read per message:
//! - `msg.type` — discriminator: `"assistant"`, `"user"`, etc.
//! - For assistant messages:
//!   - `msg.message.id` (string) — the API response id; part of
//!     the grouping key.
//!   - `msg.message.content[0].type` — must be `"tool_use"` to be
//!     considered for grouping.
//!   - `msg.message.content[0].id` — the tool-use id; used to
//!     pair with tool_results.
//!   - `msg.message.content[0].name` — the tool name; the other
//!     part of the grouping key.
//! - For user messages:
//!   - `msg.message.content[*].type === "tool_result"` — filters
//!     results out of the stream when all their tool_use_ids are
//!     grouped.
//!   - `msg.message.content[*].tool_use_id` — pairs results with
//!     the assistant's tool-use id.
//! - `msg.uuid` (string) — copied to the synthetic message's
//!   `uuid: grouped-<first.uuid>`.
//! - `msg.timestamp` — copied to the synthetic message.
//!
//! Written (synthetic messages):
//! - `type`: `"grouped_tool_use"`
//! - `toolName`, `messages[]`, `results[]`, `displayMessage`,
//!   `uuid`, `timestamp`, `messageId`.
//!
//! Gate parameter
//! ==============
//! TS reads `tool.renderGroupedToolUse` from a `Tool[]`; we take a
//! caller-supplied `HashSet<String>` of tool names. Empty set →
//! no grouping (identity). Caching behaviour (TS uses a WeakMap
//! keyed by the `Tools` array reference) is dropped — let the
//! caller cache their own set if they care; the builder is cheap.

use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

struct ToolUseInfo<'a> {
    message_id: &'a str,
    tool_use_id: &'a str,
    tool_name: &'a str,
}

fn get_tool_use_info(msg: &Value) -> Option<ToolUseInfo<'_>> {
    let obj = msg.as_object()?;
    if obj.get("type")?.as_str()? != "assistant" {
        return None;
    }
    let message = obj.get("message")?.as_object()?;
    let message_id = message.get("id")?.as_str()?;
    let content = message.get("content")?.as_array()?;
    let first = content.first()?.as_object()?;
    if first.get("type")?.as_str()? != "tool_use" {
        return None;
    }
    let tool_use_id = first.get("id")?.as_str()?;
    let tool_name = first.get("name")?.as_str()?;
    Some(ToolUseInfo {
        message_id,
        tool_use_id,
        tool_name,
    })
}

/// Group parallel tool_use messages whose tool supports grouped
/// rendering. Matches TS `applyGrouping`.
///
/// - `verbose == true` short-circuits to an identity clone.
/// - `tools_with_grouping` is the set of tool names whose
///   `renderGroupedToolUse` flag is `true` (TS `Tool.renderGroupedToolUse`).
pub fn apply_grouping(
    messages: &[Value],
    tools_with_grouping: &HashSet<String>,
    verbose: bool,
) -> Vec<Value> {
    if verbose {
        return messages.to_vec();
    }

    // First pass: group tool uses by (messageId, toolName).
    let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
    for (idx, msg) in messages.iter().enumerate() {
        let Some(info) = get_tool_use_info(msg) else {
            continue;
        };
        if !tools_with_grouping.contains(info.tool_name) {
            continue;
        }
        let key = format!("{}:{}", info.message_id, info.tool_name);
        groups.entry(key).or_default().push(idx);
    }

    // Only groups of size ≥ 2 collapse. Collect their tool_use_ids.
    let mut valid_groups: HashMap<String, Vec<usize>> = HashMap::new();
    let mut grouped_tool_use_ids: HashSet<String> = HashSet::new();
    for (key, idxs) in &groups {
        if idxs.len() < 2 {
            continue;
        }
        for &i in idxs {
            if let Some(info) = get_tool_use_info(&messages[i]) {
                grouped_tool_use_ids.insert(info.tool_use_id.to_owned());
            }
        }
        valid_groups.insert(key.clone(), idxs.clone());
    }

    // Map tool_use_id → user message index carrying that tool_result.
    let mut results_by_tool_use_id: HashMap<String, usize> = HashMap::new();
    for (idx, msg) in messages.iter().enumerate() {
        let Some(obj) = msg.as_object() else {
            continue;
        };
        if obj.get("type").and_then(Value::as_str) != Some("user") {
            continue;
        }
        let Some(content) = obj
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(Value::as_array)
        else {
            continue;
        };
        for c in content {
            let Some(c_obj) = c.as_object() else {
                continue;
            };
            if c_obj.get("type").and_then(Value::as_str) != Some("tool_result") {
                continue;
            }
            let Some(tool_use_id) = c_obj.get("tool_use_id").and_then(Value::as_str) else {
                continue;
            };
            if grouped_tool_use_ids.contains(tool_use_id) {
                // TS overwrites on collision (last-wins); Rust does the same.
                results_by_tool_use_id.insert(tool_use_id.to_owned(), idx);
            }
        }
    }

    // Second pass: emit the new stream.
    let mut result: Vec<Value> = Vec::with_capacity(messages.len());
    let mut emitted_groups: HashSet<String> = HashSet::new();

    for (idx, msg) in messages.iter().enumerate() {
        let info = get_tool_use_info(msg);

        if let Some(info_ref) = info.as_ref() {
            let key = format!("{}:{}", info_ref.message_id, info_ref.tool_name);
            if let Some(group_idxs) = valid_groups.get(&key) {
                if emitted_groups.contains(&key) {
                    // Already emitted this group — skip this member.
                    continue;
                }
                emitted_groups.insert(key);
                result.push(synthesise_group(
                    messages,
                    group_idxs,
                    info_ref.tool_name,
                    info_ref.message_id,
                    &results_by_tool_use_id,
                ));
                continue;
            }
        }

        // Skip user messages whose tool_results are ALL grouped.
        if is_user_with_all_grouped_results(msg, &grouped_tool_use_ids) {
            continue;
        }

        let _ = idx; // silence unused-warning when TS algorithm cares about order but Rust copies anyway
        result.push(msg.clone());
    }
    result
}

fn is_user_with_all_grouped_results(msg: &Value, grouped: &HashSet<String>) -> bool {
    let Some(obj) = msg.as_object() else {
        return false;
    };
    if obj.get("type").and_then(Value::as_str) != Some("user") {
        return false;
    }
    let Some(content) = obj
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(Value::as_array)
    else {
        return false;
    };
    let tool_results: Vec<&Value> = content
        .iter()
        .filter(|c| c.as_object().and_then(|o| o.get("type")?.as_str()) == Some("tool_result"))
        .collect();
    if tool_results.is_empty() {
        return false;
    }
    tool_results.iter().all(|tr| {
        let Some(id) = tr.get("tool_use_id").and_then(Value::as_str) else {
            return false;
        };
        grouped.contains(id)
    })
}

fn synthesise_group(
    messages: &[Value],
    group_idxs: &[usize],
    tool_name: &str,
    message_id: &str,
    results_by_id: &HashMap<String, usize>,
) -> Value {
    let first = &messages[group_idxs[0]];
    let grouped_assistants: Vec<Value> = group_idxs.iter().map(|&i| messages[i].clone()).collect();

    let mut results: Vec<Value> = Vec::new();
    for &i in group_idxs {
        let Some(info) = get_tool_use_info(&messages[i]) else {
            continue;
        };
        if let Some(&result_idx) = results_by_id.get(info.tool_use_id) {
            results.push(messages[result_idx].clone());
        }
    }

    let first_uuid = first
        .as_object()
        .and_then(|o| o.get("uuid"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let first_timestamp = first
        .as_object()
        .and_then(|o| o.get("timestamp"))
        .cloned()
        .unwrap_or(Value::Null);

    json!({
        "type": "grouped_tool_use",
        "toolName": tool_name,
        "messages": grouped_assistants,
        "results": results,
        "displayMessage": first.clone(),
        "uuid": format!("grouped-{first_uuid}"),
        "timestamp": first_timestamp,
        "messageId": message_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tool_use_msg(msg_id: &str, uuid: &str, tool_name: &str, tool_use_id: &str) -> Value {
        json!({
            "type": "assistant",
            "uuid": uuid,
            "timestamp": "2026-01-01T00:00:00Z",
            "message": {
                "id": msg_id,
                "content": [{
                    "type": "tool_use",
                    "id": tool_use_id,
                    "name": tool_name,
                    "input": {},
                }],
            }
        })
    }

    fn tool_result_msg(uuid: &str, tool_use_id: &str, text: &str) -> Value {
        json!({
            "type": "user",
            "uuid": uuid,
            "message": {
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": tool_use_id,
                    "content": text,
                }],
            }
        })
    }

    fn grouping(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn verbose_returns_identity() {
        let msgs = vec![tool_use_msg("m1", "u1", "Read", "t1")];
        let out = apply_grouping(&msgs, &grouping(&["Read"]), true);
        assert_eq!(out, msgs);
    }

    #[test]
    fn single_tool_use_not_grouped() {
        // One occurrence = group of 1, no collapse.
        let msgs = vec![tool_use_msg("m1", "u1", "Read", "t1")];
        let out = apply_grouping(&msgs, &grouping(&["Read"]), false);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["type"].as_str(), Some("assistant"));
    }

    #[test]
    fn two_tool_uses_same_message_same_tool_collapse() {
        let msgs = vec![
            tool_use_msg("m1", "u1", "Read", "t1"),
            tool_use_msg("m1", "u2", "Read", "t2"),
        ];
        let out = apply_grouping(&msgs, &grouping(&["Read"]), false);
        assert_eq!(out.len(), 1);
        let g = &out[0];
        assert_eq!(g["type"].as_str(), Some("grouped_tool_use"));
        assert_eq!(g["toolName"].as_str(), Some("Read"));
        assert_eq!(g["uuid"].as_str(), Some("grouped-u1"));
        assert_eq!(g["messages"].as_array().unwrap().len(), 2);
        assert_eq!(g["messageId"].as_str(), Some("m1"));
        assert_eq!(g["results"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn same_tool_different_message_does_not_collapse() {
        // TS groups by messageId + toolName — different messageIds stay separate.
        let msgs = vec![
            tool_use_msg("m1", "u1", "Read", "t1"),
            tool_use_msg("m2", "u2", "Read", "t2"),
        ];
        let out = apply_grouping(&msgs, &grouping(&["Read"]), false);
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|m| m["type"].as_str() == Some("assistant")));
    }

    #[test]
    fn different_tools_same_message_do_not_collapse() {
        let msgs = vec![
            tool_use_msg("m1", "u1", "Read", "t1"),
            tool_use_msg("m1", "u2", "Grep", "t2"),
        ];
        let out = apply_grouping(&msgs, &grouping(&["Read", "Grep"]), false);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn tool_not_in_grouping_set_does_not_collapse() {
        let msgs = vec![
            tool_use_msg("m1", "u1", "Bash", "t1"),
            tool_use_msg("m1", "u2", "Bash", "t2"),
        ];
        // Grouping set does NOT include Bash → pass through.
        let out = apply_grouping(&msgs, &grouping(&["Read"]), false);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn tool_results_attach_to_group() {
        let msgs = vec![
            tool_use_msg("m1", "u1", "Read", "t1"),
            tool_use_msg("m1", "u2", "Read", "t2"),
            tool_result_msg("r1", "t1", "result one"),
            tool_result_msg("r2", "t2", "result two"),
        ];
        let out = apply_grouping(&msgs, &grouping(&["Read"]), false);
        // 1 grouped + 0 results (results are absorbed into the group's
        // `results` array AND skipped in the output stream because all
        // their tool_results are grouped).
        assert_eq!(out.len(), 1);
        let g = &out[0];
        assert_eq!(g["type"].as_str(), Some("grouped_tool_use"));
        assert_eq!(g["results"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn user_with_mixed_grouped_and_ungrouped_results_is_kept() {
        // One result is for a grouped tool, one is NOT — the user
        // message stays in the output stream because not all of its
        // tool_results are grouped.
        let msgs = vec![
            tool_use_msg("m1", "u1", "Read", "t1"),
            tool_use_msg("m1", "u2", "Read", "t2"),
            json!({
                "type": "user",
                "uuid": "r-mixed",
                "message": {
                    "role": "user",
                    "content": [
                        { "type": "tool_result", "tool_use_id": "t1", "content": "grouped result" },
                        { "type": "tool_result", "tool_use_id": "t-ungrouped", "content": "other" },
                    ],
                }
            }),
        ];
        let out = apply_grouping(&msgs, &grouping(&["Read"]), false);
        // Grouped tool-use + the mixed-result user msg (2 total).
        assert_eq!(out.len(), 2);
        assert_eq!(out[0]["type"].as_str(), Some("grouped_tool_use"));
        assert_eq!(out[1]["uuid"].as_str(), Some("r-mixed"));
    }

    #[test]
    fn three_tool_uses_collapse_with_order_preserved() {
        let msgs = vec![
            tool_use_msg("m1", "u1", "Read", "t1"),
            tool_use_msg("m1", "u2", "Read", "t2"),
            tool_use_msg("m1", "u3", "Read", "t3"),
        ];
        let out = apply_grouping(&msgs, &grouping(&["Read"]), false);
        assert_eq!(out.len(), 1);
        let members = out[0]["messages"].as_array().unwrap();
        assert_eq!(members.len(), 3);
        // Order preserved — message iteration visits u1 → u2 → u3.
        assert_eq!(members[0]["uuid"].as_str(), Some("u1"));
        assert_eq!(members[2]["uuid"].as_str(), Some("u3"));
    }

    #[test]
    fn non_tool_use_messages_pass_through_unchanged() {
        let msgs = vec![
            json!({ "type": "user", "uuid": "plain-user" }),
            json!({ "type": "system", "subtype": "info", "uuid": "sys1" }),
        ];
        let out = apply_grouping(&msgs, &grouping(&["Read"]), false);
        assert_eq!(out, msgs);
    }

    #[test]
    fn group_display_message_is_first_member() {
        let msgs = vec![
            tool_use_msg("m1", "u-first", "Read", "t1"),
            tool_use_msg("m1", "u-second", "Read", "t2"),
        ];
        let out = apply_grouping(&msgs, &grouping(&["Read"]), false);
        let display = &out[0]["displayMessage"];
        assert_eq!(display["uuid"].as_str(), Some("u-first"));
    }

    #[test]
    fn empty_grouping_set_is_identity() {
        let msgs = vec![
            tool_use_msg("m1", "u1", "Read", "t1"),
            tool_use_msg("m1", "u2", "Read", "t2"),
        ];
        let out = apply_grouping(&msgs, &HashSet::new(), false);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn empty_input_returns_empty() {
        let out = apply_grouping(&[], &grouping(&["Read"]), false);
        assert!(out.is_empty());
    }
}
