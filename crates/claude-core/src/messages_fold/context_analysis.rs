//! Token-usage breakdown of a message stream for context-budget analytics.
//!
//! Port of TS `utils/contextAnalysis.ts:1-272`.
//!
//! **Reconstructed types disclaimer.** The TS file imports
//! `Message`, `AssistantMessage`, and `UserMessage` from
//! `src/types/message.ts`, **which is missing from the leaked
//! source snapshot**. Rather than reverse-engineer the type graph,
//! this port works on `serde_json::Value` streams and documents
//! the fields read below.
//!
//! Also bypasses one dep
//! =====================
//! TS runs `normalizeMessagesForAPI(messages)` before walking the
//! stream (merges consecutive same-role user messages, drops empties).
//! The existing Rust `api::normalize::normalize_messages` operates on
//! a different on-the-wire shape (`{role, content}` not
//! `{type, message: {role, content}}`). Rather than divert to port
//! that, this function takes messages AS-IS and documents that the
//! caller should pre-normalise if they want parity with TS's
//! merging behaviour. Every downstream field access is defensive
//! (null-tolerant), so an un-merged stream yields the same per-
//! message stats; the only difference is that two consecutive user
//! messages would be counted separately rather than merged into one
//! turn. Acceptable for the analytics use case.
//!
//! Fields touched
//! ==============
//! - `msg.type`: `"attachment"` / `"user"` / `"assistant"` /
//!   `"system"` / others — discriminator.
//! - `msg.attachment.type` (for attachment msgs): attachment subtype
//!   counted into `attachments`.
//! - `msg.message.content`: string OR array of content blocks.
//! - Content block fields:
//!   - `block.type`: `"text"`, `"tool_use"`, `"tool_result"`,
//!     `"image"`, `"thinking"`, `"redacted_thinking"`,
//!     `"server_tool_use"`, `"web_search_tool_result"`,
//!     `"search_result"`, `"document"`,
//!     `"code_execution_tool_result"`, `"mcp_tool_use"`,
//!     `"mcp_tool_result"`, `"container_upload"`,
//!     `"web_fetch_tool_result"`,
//!     `"bash_code_execution_tool_result"`,
//!     `"text_editor_code_execution_tool_result"`,
//!     `"tool_search_tool_result"`, `"compaction"`.
//!   - `block.text` (text blocks)
//!   - `block.name`, `block.id`, `block.input.file_path`
//!     (tool_use blocks)
//!   - `block.tool_use_id` (tool_result blocks).

use crate::string_utils::rough_token_count_estimation;
use serde_json::Value;
use std::collections::HashMap;

/// Output shape — matches TS `TokenStats`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TokenStats {
    pub tool_requests: HashMap<String, u64>,
    pub tool_results: HashMap<String, u64>,
    pub human_messages: u64,
    pub assistant_messages: u64,
    pub local_command_outputs: u64,
    pub other: u64,
    pub attachments: HashMap<String, u64>,
    /// File path → (count, tokens-wasted-by-dup) for files read more
    /// than once.
    pub duplicate_file_reads: HashMap<String, DuplicateRead>,
    pub total: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateRead {
    pub count: u32,
    pub tokens: u64,
}

/// Walk a message stream and compute a token breakdown. Matches TS
/// `analyzeContext`.
pub fn analyze_context(messages: &[Value]) -> TokenStats {
    let mut stats = TokenStats::default();
    let mut tool_ids_to_names: HashMap<String, String> = HashMap::new();
    let mut read_tool_paths: HashMap<String, String> = HashMap::new();
    // path → (count, total_tokens)
    let mut file_reads: HashMap<String, (u32, u64)> = HashMap::new();

    // First pass: count attachments by subtype.
    for msg in messages {
        if let Some(obj) = msg.as_object() {
            if obj.get("type").and_then(Value::as_str) == Some("attachment") {
                let kind = obj
                    .get("attachment")
                    .and_then(|a| a.get("type"))
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_owned();
                *stats.attachments.entry(kind).or_insert(0) += 1;
            }
        }
    }

    // Second pass: walk message.content, accumulate token counts.
    for msg in messages {
        let Some(obj) = msg.as_object() else { continue };
        let msg_type = obj.get("type").and_then(Value::as_str).unwrap_or("");
        if msg_type != "user" && msg_type != "assistant" {
            // Attachment / system / progress — not counted here.
            continue;
        }
        let Some(message) = obj.get("message") else {
            continue;
        };
        let Some(content) = message.get("content") else {
            continue;
        };

        if let Some(s) = content.as_str() {
            let tokens = rough_token_count_estimation(s) as u64;
            stats.total += tokens;
            if msg_type == "user" && s.contains("local-command-stdout") {
                stats.local_command_outputs += tokens;
            } else if msg_type == "user" {
                stats.human_messages += tokens;
            } else {
                stats.assistant_messages += tokens;
            }
            continue;
        }

        if let Some(arr) = content.as_array() {
            for block in arr {
                process_block(
                    block,
                    msg_type,
                    &mut stats,
                    &mut tool_ids_to_names,
                    &mut read_tool_paths,
                    &mut file_reads,
                );
            }
        }
    }

    // Compute duplicate-read wastage.
    for (path, (count, total_tokens)) in file_reads {
        if count > 1 {
            let avg_tokens_per_read = total_tokens / count as u64;
            let duplicate_tokens = avg_tokens_per_read * (count as u64 - 1);
            stats.duplicate_file_reads.insert(
                path,
                DuplicateRead {
                    count,
                    tokens: duplicate_tokens,
                },
            );
        }
    }

    stats
}

fn process_block(
    block: &Value,
    msg_type: &str,
    stats: &mut TokenStats,
    tool_ids: &mut HashMap<String, String>,
    read_tool_paths: &mut HashMap<String, String>,
    file_reads: &mut HashMap<String, (u32, u64)>,
) {
    // TS counts tokens on the JSON-serialised block. Rust does the same
    // via `serde_json::to_string` + `rough_token_count_estimation`.
    let serialised = match serde_json::to_string(block) {
        Ok(s) => s,
        Err(_) => return,
    };
    let tokens = rough_token_count_estimation(&serialised) as u64;
    stats.total += tokens;

    let Some(obj) = block.as_object() else {
        return;
    };
    let Some(kind) = obj.get("type").and_then(Value::as_str) else {
        return;
    };

    match kind {
        "text" => {
            let text = obj.get("text").and_then(Value::as_str).unwrap_or("");
            if msg_type == "user" && text.contains("local-command-stdout") {
                stats.local_command_outputs += tokens;
            } else if msg_type == "user" {
                stats.human_messages += tokens;
            } else {
                stats.assistant_messages += tokens;
            }
        }
        "tool_use" => {
            let Some(name) = obj.get("name").and_then(Value::as_str) else {
                return;
            };
            let Some(id) = obj.get("id").and_then(Value::as_str) else {
                return;
            };
            let tool_name = if name.is_empty() { "unknown" } else { name };
            *stats.tool_requests.entry(tool_name.to_owned()).or_insert(0) += tokens;
            tool_ids.insert(id.to_owned(), tool_name.to_owned());

            // Track Read tool file paths for dup-read tracking.
            if tool_name == "Read" {
                if let Some(path) = obj
                    .get("input")
                    .and_then(|i| i.as_object())
                    .and_then(|i| i.get("file_path"))
                    .and_then(Value::as_str)
                {
                    read_tool_paths.insert(id.to_owned(), path.to_owned());
                }
            }
        }
        "tool_result" => {
            let Some(tool_use_id) = obj.get("tool_use_id").and_then(Value::as_str) else {
                return;
            };
            let tool_name = tool_ids
                .get(tool_use_id)
                .cloned()
                .unwrap_or_else(|| "unknown".to_owned());
            *stats.tool_results.entry(tool_name.clone()).or_insert(0) += tokens;

            if tool_name == "Read" {
                if let Some(path) = read_tool_paths.get(tool_use_id).cloned() {
                    let entry = file_reads.entry(path).or_insert((0, 0));
                    entry.0 += 1;
                    entry.1 += tokens;
                }
            }
        }
        // All the rest — TS case fall-through to `stats.other += tokens`.
        "image"
        | "server_tool_use"
        | "web_search_tool_result"
        | "search_result"
        | "document"
        | "thinking"
        | "redacted_thinking"
        | "code_execution_tool_result"
        | "mcp_tool_use"
        | "mcp_tool_result"
        | "container_upload"
        | "web_fetch_tool_result"
        | "bash_code_execution_tool_result"
        | "text_editor_code_execution_tool_result"
        | "tool_search_tool_result"
        | "compaction" => {
            stats.other += tokens;
        }
        _ => {
            // Unknown kinds: TS falls through (default case with no
            // matching branch leaves the stat untouched). Tokens still
            // count into `total` above, matching TS.
        }
    }
}

/// Build a flat metrics map suitable for Statsig / analytics. Matches
/// TS `tokenStatsToStatsigMetrics`.
pub fn token_stats_to_statsig_metrics(stats: &TokenStats) -> HashMap<String, i64> {
    let mut m: HashMap<String, i64> = HashMap::new();
    m.insert("total_tokens".into(), stats.total as i64);
    m.insert("human_message_tokens".into(), stats.human_messages as i64);
    m.insert(
        "assistant_message_tokens".into(),
        stats.assistant_messages as i64,
    );
    m.insert(
        "local_command_output_tokens".into(),
        stats.local_command_outputs as i64,
    );
    m.insert("other_tokens".into(), stats.other as i64);

    for (k, v) in &stats.attachments {
        m.insert(format!("attachment_{k}_count"), *v as i64);
    }
    for (k, v) in &stats.tool_requests {
        m.insert(format!("tool_request_{k}_tokens"), *v as i64);
    }
    for (k, v) in &stats.tool_results {
        m.insert(format!("tool_result_{k}_tokens"), *v as i64);
    }

    let duplicate_total: u64 = stats.duplicate_file_reads.values().map(|d| d.tokens).sum();
    m.insert("duplicate_read_tokens".into(), duplicate_total as i64);
    m.insert(
        "duplicate_read_file_count".into(),
        stats.duplicate_file_reads.len() as i64,
    );

    if stats.total > 0 {
        let pct = |n: u64| ((n as f64 / stats.total as f64) * 100.0).round() as i64;
        m.insert("human_message_percent".into(), pct(stats.human_messages));
        m.insert(
            "assistant_message_percent".into(),
            pct(stats.assistant_messages),
        );
        m.insert(
            "local_command_output_percent".into(),
            pct(stats.local_command_outputs),
        );
        m.insert("duplicate_read_percent".into(), pct(duplicate_total));

        let tool_request_total: u64 = stats.tool_requests.values().sum();
        let tool_result_total: u64 = stats.tool_results.values().sum();
        m.insert("tool_request_percent".into(), pct(tool_request_total));
        m.insert("tool_result_percent".into(), pct(tool_result_total));

        for (k, v) in &stats.tool_requests {
            m.insert(format!("tool_request_{k}_percent"), pct(*v));
        }
        for (k, v) in &stats.tool_results {
            m.insert(format!("tool_result_{k}_percent"), pct(*v));
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn empty_input_empty_stats() {
        let s = analyze_context(&[]);
        assert_eq!(s.total, 0);
        assert!(s.attachments.is_empty());
    }

    #[test]
    fn attachment_counts_by_subtype() {
        let msgs = vec![
            json!({ "type": "attachment", "attachment": { "type": "file" } }),
            json!({ "type": "attachment", "attachment": { "type": "file" } }),
            json!({ "type": "attachment", "attachment": { "type": "hook_output" } }),
        ];
        let s = analyze_context(&msgs);
        assert_eq!(s.attachments.get("file"), Some(&2));
        assert_eq!(s.attachments.get("hook_output"), Some(&1));
    }

    #[test]
    fn unknown_attachment_uses_unknown_bucket() {
        let msgs = vec![json!({ "type": "attachment", "attachment": {} })];
        let s = analyze_context(&msgs);
        assert_eq!(s.attachments.get("unknown"), Some(&1));
    }

    #[test]
    fn user_text_counts_as_human_message() {
        let msgs = vec![json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": [{ "type": "text", "text": "hello world" }],
            }
        })];
        let s = analyze_context(&msgs);
        assert!(s.human_messages > 0);
        assert_eq!(s.assistant_messages, 0);
        assert!(s.total >= s.human_messages);
    }

    #[test]
    fn local_command_stdout_text_routes_to_local_bucket() {
        let msgs = vec![json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": [{
                    "type": "text",
                    "text": "<local-command-stdout>hi</local-command-stdout>",
                }],
            }
        })];
        let s = analyze_context(&msgs);
        assert!(s.local_command_outputs > 0);
        assert_eq!(s.human_messages, 0);
    }

    #[test]
    fn assistant_text_counts_as_assistant() {
        let msgs = vec![json!({
            "type": "assistant",
            "message": {
                "role": "assistant",
                "content": [{ "type": "text", "text": "response" }],
            }
        })];
        let s = analyze_context(&msgs);
        assert!(s.assistant_messages > 0);
        assert_eq!(s.human_messages, 0);
    }

    #[test]
    fn tool_use_increments_tool_requests() {
        let msgs = vec![json!({
            "type": "assistant",
            "message": {
                "content": [{
                    "type": "tool_use",
                    "id": "t1",
                    "name": "Read",
                    "input": { "file_path": "/tmp/a.txt" },
                }]
            }
        })];
        let s = analyze_context(&msgs);
        assert!(s.tool_requests.get("Read").copied().unwrap_or(0) > 0);
    }

    #[test]
    fn tool_result_increments_tool_results_by_name() {
        let msgs = vec![
            json!({
                "type": "assistant",
                "message": {
                    "content": [{
                        "type": "tool_use",
                        "id": "t1",
                        "name": "Read",
                        "input": { "file_path": "/tmp/a.txt" },
                    }]
                }
            }),
            json!({
                "type": "user",
                "message": {
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": "t1",
                        "content": "file contents here",
                    }]
                }
            }),
        ];
        let s = analyze_context(&msgs);
        assert!(s.tool_results.get("Read").copied().unwrap_or(0) > 0);
    }

    #[test]
    fn duplicate_read_tracked() {
        // Same file path read twice → duplicate_file_reads populated.
        let use_read = |id: &str, path: &str| -> Value {
            json!({
                "type": "assistant",
                "message": {
                    "content": [{
                        "type": "tool_use",
                        "id": id,
                        "name": "Read",
                        "input": { "file_path": path },
                    }]
                }
            })
        };
        let result = |id: &str, txt: &str| -> Value {
            json!({
                "type": "user",
                "message": {
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": id,
                        "content": txt,
                    }]
                }
            })
        };
        let msgs = vec![
            use_read("t1", "/tmp/a.txt"),
            result("t1", "contents once"),
            use_read("t2", "/tmp/a.txt"),
            result("t2", "contents twice"),
        ];
        let s = analyze_context(&msgs);
        let dup = s.duplicate_file_reads.get("/tmp/a.txt").unwrap();
        assert_eq!(dup.count, 2);
        assert!(dup.tokens > 0);
    }

    #[test]
    fn other_block_types_go_to_other_bucket() {
        let msgs = vec![json!({
            "type": "assistant",
            "message": {
                "content": [{ "type": "thinking", "thinking": "inner monologue" }]
            }
        })];
        let s = analyze_context(&msgs);
        assert!(s.other > 0);
        assert_eq!(s.human_messages, 0);
        assert_eq!(s.assistant_messages, 0);
    }

    #[test]
    fn statsig_metrics_flat_shape() {
        let mut stats = TokenStats {
            total: 100,
            human_messages: 50,
            assistant_messages: 30,
            local_command_outputs: 10,
            other: 10,
            ..Default::default()
        };
        stats.tool_requests.insert("Read".into(), 25);
        stats.tool_results.insert("Read".into(), 20);

        let m = token_stats_to_statsig_metrics(&stats);
        assert_eq!(m.get("total_tokens"), Some(&100));
        assert_eq!(m.get("human_message_tokens"), Some(&50));
        assert_eq!(m.get("tool_request_Read_tokens"), Some(&25));
        // Percents computed correctly (50/100 * 100 = 50).
        assert_eq!(m.get("human_message_percent"), Some(&50));
        assert_eq!(m.get("tool_request_Read_percent"), Some(&25));
    }

    #[test]
    fn statsig_skips_percents_when_total_zero() {
        let stats = TokenStats::default();
        let m = token_stats_to_statsig_metrics(&stats);
        assert!(!m.contains_key("human_message_percent"));
        assert_eq!(m.get("total_tokens"), Some(&0));
    }

    #[test]
    fn unnamed_tool_use_with_id_still_tracked() {
        // TS has `toolName = block.name || 'unknown'` — an empty name
        // string should be treated as unknown.
        let msgs = vec![json!({
            "type": "assistant",
            "message": {
                "content": [{
                    "type": "tool_use",
                    "id": "t1",
                    "name": "",
                    "input": {},
                }]
            }
        })];
        let s = analyze_context(&msgs);
        assert!(s.tool_requests.get("unknown").copied().unwrap_or(0) > 0);
    }
}
