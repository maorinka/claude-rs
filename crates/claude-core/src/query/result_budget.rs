//! Tool-result persistence for the query loop.
//!
//! Mirrors the hot path of TS `utils/toolResultStorage.ts`: large textual tool
//! results are written under the session's `tool-results` directory and the
//! model receives a tagged preview pointing at the file. The aggregate
//! per-message replacement state is kept in memory like TS, but transcript
//! serialization for resume still depends on the session log layer.

use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

pub const DEFAULT_MAX_RESULT_SIZE_CHARS: usize = 50_000;
pub const MAX_TOOL_RESULT_TOKENS: usize = 100_000;
pub const BYTES_PER_TOKEN: usize = 4;
pub const MAX_TOOL_RESULT_BYTES: usize = MAX_TOOL_RESULT_TOKENS * BYTES_PER_TOKEN;
pub const MAX_TOOL_RESULTS_PER_MESSAGE_CHARS: usize = 200_000;
pub const PREVIEW_SIZE_BYTES: usize = 2_000;
pub const TOOL_RESULTS_SUBDIR: &str = "tool-results";
pub const PERSISTED_OUTPUT_TAG: &str = "<persisted-output>";
pub const PERSISTED_OUTPUT_CLOSING_TAG: &str = "</persisted-output>";
pub const TOOL_RESULT_CLEARED_MESSAGE: &str = "[Old tool result content cleared]";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedToolResult {
    pub filepath: PathBuf,
    pub original_size: usize,
    pub is_json: bool,
    pub preview: String,
    pub has_more: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    pub preview: String,
    pub has_more: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContentReplacementState {
    seen_ids: HashSet<String>,
    replacements: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ContentReplacementRecord {
    #[serde(rename = "tool-result")]
    ToolResult {
        #[serde(rename = "toolUseId")]
        tool_use_id: String,
        replacement: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetApplication {
    pub messages: Vec<Value>,
    pub newly_replaced: Vec<ContentReplacementRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ToolResultCandidate {
    tool_use_id: String,
    content: Value,
    size: usize,
}

impl ContentReplacementState {
    pub fn reconstruct_from_messages_and_records(
        messages: &[Value],
        records: &[ContentReplacementRecord],
    ) -> Self {
        let mut state = Self::default();
        for message in messages {
            for candidate in collect_candidates_from_message(message) {
                state.seen_ids.insert(candidate.tool_use_id);
            }
        }
        for record in records {
            let ContentReplacementRecord::ToolResult {
                tool_use_id,
                replacement,
            } = record;
            state.seen_ids.insert(tool_use_id.clone());
            state
                .replacements
                .insert(tool_use_id.clone(), replacement.clone());
        }
        state
    }
}

/// Same threshold rule as TS `getPersistenceThreshold` without GrowthBook
/// overrides: infinite tool caps opt out, otherwise clamp to the global cap.
pub fn persistence_threshold(declared_max_result_size_chars: usize) -> usize {
    if declared_max_result_size_chars == usize::MAX {
        usize::MAX
    } else {
        declared_max_result_size_chars.min(DEFAULT_MAX_RESULT_SIZE_CHARS)
    }
}

/// Process a pre-mapped tool_result `content` for inclusion in conversation
/// state. Returns the original value when TS would skip persistence.
pub fn process_tool_result_content(
    session_id: &str,
    tool_use_id: &str,
    tool_name: &str,
    declared_max_result_size_chars: usize,
    content: Value,
) -> Value {
    if is_tool_result_content_empty(&content) {
        return Value::String(format!("({tool_name} completed with no output)"));
    }

    if has_image_block(&content) {
        return content;
    }

    let threshold = persistence_threshold(declared_max_result_size_chars);
    if content_size(&content) <= threshold {
        return content;
    }

    match persist_tool_result(session_id, tool_use_id, &content) {
        Ok(result) => Value::String(build_large_tool_result_message(&result)),
        Err(_) => content,
    }
}

/// Enforce TS's aggregate per-message tool result budget. Previously-seen
/// decisions are frozen to keep prompt-cache prefixes stable across turns.
pub fn apply_tool_result_budget(
    session_id: &str,
    messages: &[Value],
    state: &mut ContentReplacementState,
    skip_tool_use_ids: &HashSet<String>,
) -> BudgetApplication {
    let candidates_by_message = collect_candidates_by_message(messages);
    let mut replacement_map: HashMap<String, String> = HashMap::new();
    let mut to_persist: Vec<ToolResultCandidate> = Vec::new();
    let mut newly_replaced = Vec::new();

    for candidates in candidates_by_message {
        let mut fresh = Vec::new();
        let mut frozen_size = 0usize;

        for candidate in candidates {
            if let Some(replacement) = state.replacements.get(&candidate.tool_use_id) {
                replacement_map.insert(candidate.tool_use_id.clone(), replacement.clone());
            } else if state.seen_ids.contains(&candidate.tool_use_id) {
                frozen_size += candidate.size;
            } else if skip_tool_use_ids.contains(&candidate.tool_use_id) {
                state.seen_ids.insert(candidate.tool_use_id.clone());
            } else {
                fresh.push(candidate);
            }
        }

        if fresh.is_empty() {
            continue;
        }

        let fresh_size: usize = fresh.iter().map(|candidate| candidate.size).sum();
        let selected = if frozen_size + fresh_size > MAX_TOOL_RESULTS_PER_MESSAGE_CHARS {
            select_fresh_to_replace(
                fresh.as_slice(),
                frozen_size,
                MAX_TOOL_RESULTS_PER_MESSAGE_CHARS,
            )
        } else {
            Vec::new()
        };
        let selected_ids: HashSet<String> = selected
            .iter()
            .map(|candidate| candidate.tool_use_id.clone())
            .collect();

        for candidate in fresh {
            if selected_ids.contains(&candidate.tool_use_id) {
                to_persist.push(candidate);
            } else {
                state.seen_ids.insert(candidate.tool_use_id);
            }
        }
    }

    for candidate in to_persist {
        state.seen_ids.insert(candidate.tool_use_id.clone());
        let Ok(result) =
            persist_tool_result(session_id, &candidate.tool_use_id, &candidate.content)
        else {
            continue;
        };
        let replacement = build_large_tool_result_message(&result);
        state
            .replacements
            .insert(candidate.tool_use_id.clone(), replacement.clone());
        newly_replaced.push(ContentReplacementRecord::ToolResult {
            tool_use_id: candidate.tool_use_id.clone(),
            replacement: replacement.clone(),
        });
        replacement_map.insert(candidate.tool_use_id, replacement);
    }

    if replacement_map.is_empty() {
        return BudgetApplication {
            messages: messages.to_vec(),
            newly_replaced,
        };
    }
    BudgetApplication {
        messages: replace_tool_result_contents(messages, &replacement_map),
        newly_replaced,
    }
}

pub fn persist_tool_result(
    session_id: &str,
    tool_use_id: &str,
    content: &Value,
) -> std::io::Result<PersistedToolResult> {
    let Some((content_str, is_json)) = persistable_content_string(content) else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Cannot persist tool results containing non-text content",
        ));
    };

    let dir = tool_results_dir(session_id)?;
    std::fs::create_dir_all(&dir)?;
    let filepath = dir.join(format!(
        "{}.{}",
        tool_use_id,
        if is_json { "json" } else { "txt" }
    ));

    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&filepath)
    {
        Ok(mut file) => file.write_all(content_str.as_bytes())?,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }

    let Preview { preview, has_more } = generate_preview(&content_str, PREVIEW_SIZE_BYTES);
    Ok(PersistedToolResult {
        filepath,
        original_size: content_str.len(),
        is_json,
        preview,
        has_more,
    })
}

pub fn build_large_tool_result_message(result: &PersistedToolResult) -> String {
    let mut message = format!("{PERSISTED_OUTPUT_TAG}\n");
    message.push_str(&format!(
        "Output too large ({}). Full output saved to: {}\n\n",
        format_file_size(result.original_size),
        result.filepath.display()
    ));
    message.push_str(&format!(
        "Preview (first {}):\n",
        format_file_size(PREVIEW_SIZE_BYTES)
    ));
    message.push_str(&result.preview);
    if result.has_more {
        message.push_str("\n...\n");
    } else {
        message.push('\n');
    }
    message.push_str(PERSISTED_OUTPUT_CLOSING_TAG);
    message
}

pub fn generate_preview(content: &str, max_bytes: usize) -> Preview {
    if content.len() <= max_bytes {
        return Preview {
            preview: content.to_string(),
            has_more: false,
        };
    }

    let mut limit = max_bytes.min(content.len());
    while limit > 0 && !content.is_char_boundary(limit) {
        limit -= 1;
    }
    let truncated = &content[..limit];
    let last_newline = truncated.rfind('\n');
    let cut_point = last_newline
        .filter(|idx| *idx > max_bytes / 2 && content.is_char_boundary(*idx))
        .unwrap_or(limit);

    Preview {
        preview: content[..cut_point].to_string(),
        has_more: true,
    }
}

pub fn is_tool_result_content_empty(content: &Value) -> bool {
    match content {
        Value::Null => true,
        Value::String(text) => text.trim().is_empty(),
        Value::Array(blocks) if blocks.is_empty() => true,
        Value::Array(blocks) => blocks.iter().all(|block| {
            block.get("type").and_then(Value::as_str) == Some("text")
                && block
                    .get("text")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .unwrap_or("")
                    .is_empty()
        }),
        _ => false,
    }
}

pub fn has_image_block(content: &Value) -> bool {
    matches!(content, Value::Array(blocks) if blocks.iter().any(|block| {
        block.get("type").and_then(Value::as_str) == Some("image")
    }))
}

pub fn content_size(content: &Value) -> usize {
    match content {
        Value::String(text) => text.len(),
        Value::Array(blocks) => blocks
            .iter()
            .map(|block| {
                if block.get("type").and_then(Value::as_str) == Some("text") {
                    block
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .len()
                } else {
                    0
                }
            })
            .sum(),
        _ => 0,
    }
}

fn collect_candidates_by_message(messages: &[Value]) -> Vec<Vec<ToolResultCandidate>> {
    let mut groups = Vec::new();
    let mut current = Vec::new();

    for message in messages {
        match message.get("role").and_then(Value::as_str) {
            Some("user") => current.extend(collect_candidates_from_message(message)),
            Some("assistant") => {
                if !current.is_empty() {
                    groups.push(std::mem::take(&mut current));
                }
            }
            _ => {}
        }
    }

    if !current.is_empty() {
        groups.push(current);
    }
    groups
}

fn collect_candidates_from_message(message: &Value) -> Vec<ToolResultCandidate> {
    let Some(content) = message.get("content").and_then(Value::as_array) else {
        return Vec::new();
    };
    content
        .iter()
        .filter_map(|block| {
            if block.get("type").and_then(Value::as_str) != Some("tool_result") {
                return None;
            }
            let content = block.get("content")?;
            if is_tool_result_content_empty(content)
                || has_image_block(content)
                || is_content_already_compacted(content)
            {
                return None;
            }
            Some(ToolResultCandidate {
                tool_use_id: block.get("tool_use_id")?.as_str()?.to_string(),
                content: content.clone(),
                size: content_size(content),
            })
        })
        .collect()
}

fn is_content_already_compacted(content: &Value) -> bool {
    content
        .as_str()
        .map(|text| text.starts_with(PERSISTED_OUTPUT_TAG))
        .unwrap_or(false)
}

fn select_fresh_to_replace(
    fresh: &[ToolResultCandidate],
    frozen_size: usize,
    limit: usize,
) -> Vec<ToolResultCandidate> {
    let mut sorted = fresh.to_vec();
    sorted.sort_by(|a, b| b.size.cmp(&a.size));

    let mut selected = Vec::new();
    let mut remaining = frozen_size + fresh.iter().map(|candidate| candidate.size).sum::<usize>();
    for candidate in sorted {
        if remaining <= limit {
            break;
        }
        remaining = remaining.saturating_sub(candidate.size);
        selected.push(candidate);
    }
    selected
}

fn replace_tool_result_contents(
    messages: &[Value],
    replacement_map: &HashMap<String, String>,
) -> Vec<Value> {
    messages
        .iter()
        .map(|message| {
            if message.get("role").and_then(Value::as_str) != Some("user") {
                return message.clone();
            }
            let Some(content) = message.get("content").and_then(Value::as_array) else {
                return message.clone();
            };
            let needs_replace = content.iter().any(|block| {
                block.get("type").and_then(Value::as_str) == Some("tool_result")
                    && block
                        .get("tool_use_id")
                        .and_then(Value::as_str)
                        .map(|id| replacement_map.contains_key(id))
                        .unwrap_or(false)
            });
            if !needs_replace {
                return message.clone();
            }

            let mut next = message.clone();
            let Some(next_content) = next.get_mut("content").and_then(Value::as_array_mut) else {
                return message.clone();
            };
            for block in next_content {
                if block.get("type").and_then(Value::as_str) != Some("tool_result") {
                    continue;
                }
                let replacement = block
                    .get("tool_use_id")
                    .and_then(Value::as_str)
                    .and_then(|id| replacement_map.get(id))
                    .cloned();
                if let Some(replacement) = replacement {
                    block["content"] = Value::String(replacement);
                }
            }
            next
        })
        .collect()
}

fn persistable_content_string(content: &Value) -> Option<(String, bool)> {
    match content {
        Value::String(text) => Some((text.clone(), false)),
        Value::Array(blocks) => {
            if blocks
                .iter()
                .any(|block| block.get("type").and_then(Value::as_str) != Some("text"))
            {
                return None;
            }
            serde_json::to_string_pretty(content)
                .ok()
                .map(|text| (text, true))
        }
        _ => None,
    }
}

fn tool_results_dir(session_id: &str) -> std::io::Result<PathBuf> {
    let home = dirs::home_dir()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "home not found"))?;
    let cwd = std::env::current_dir()?;
    Ok(home
        .join(".claude")
        .join("projects")
        .join(sanitize_session_path(&cwd.display().to_string()))
        .join(session_id)
        .join(TOOL_RESULTS_SUBDIR))
}

fn sanitize_session_path(name: &str) -> String {
    name.chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect()
}

fn format_file_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} bytes")
    } else if bytes < 1024 * 1024 {
        format_decimal_unit(bytes as f64 / 1024.0, "KB")
    } else if bytes < 1024 * 1024 * 1024 {
        format_decimal_unit(bytes as f64 / 1024.0 / 1024.0, "MB")
    } else {
        format_decimal_unit(bytes as f64 / 1024.0 / 1024.0 / 1024.0, "GB")
    }
}

fn format_decimal_unit(value: f64, unit: &str) -> String {
    let formatted = format!("{value:.1}");
    let trimmed = formatted.strip_suffix(".0").unwrap_or(&formatted);
    format!("{trimmed}{unit}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_temp_home_and_cwd<T>(f: impl FnOnce() -> T) -> T {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let home = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        let old_home = std::env::var_os("HOME");
        let old_cwd =
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")));

        struct RestoreEnv {
            old_home: Option<std::ffi::OsString>,
            old_cwd: PathBuf,
        }

        impl Drop for RestoreEnv {
            fn drop(&mut self) {
                if let Some(old_home) = &self.old_home {
                    std::env::set_var("HOME", old_home);
                } else {
                    std::env::remove_var("HOME");
                }
                let _ = std::env::set_current_dir(&self.old_cwd);
            }
        }

        let _restore = RestoreEnv { old_home, old_cwd };
        std::env::set_var("HOME", home.path());
        std::env::set_current_dir(cwd.path()).unwrap();
        f()
    }

    fn persisted_path_from_message(message: &str) -> PathBuf {
        let line = message
            .lines()
            .find(|line| line.contains("Full output saved to: "))
            .unwrap();
        PathBuf::from(line.split("Full output saved to: ").nth(1).unwrap())
    }

    #[test]
    fn threshold_clamps_finite_tool_caps() {
        assert_eq!(persistence_threshold(100_000), 50_000);
        assert_eq!(persistence_threshold(30_000), 30_000);
        assert_eq!(persistence_threshold(usize::MAX), usize::MAX);
    }

    #[test]
    fn preview_prefers_recent_newline() {
        let content = format!("{}\n{}", "a".repeat(1_700), "b".repeat(1_000));
        let out = generate_preview(&content, PREVIEW_SIZE_BYTES);
        assert_eq!(out.preview, "a".repeat(1_700));
        assert!(out.has_more);
    }

    #[test]
    fn preview_falls_back_to_exact_limit() {
        let content = "x".repeat(3_000);
        let out = generate_preview(&content, PREVIEW_SIZE_BYTES);
        assert_eq!(out.preview.len(), PREVIEW_SIZE_BYTES);
        assert!(out.has_more);
    }

    #[test]
    fn empty_result_gets_ts_marker() {
        let content = process_tool_result_content(
            "session",
            "toolu_1",
            "Bash",
            30_000,
            Value::String("  ".into()),
        );
        assert_eq!(
            content,
            Value::String("(Bash completed with no output)".into())
        );
    }

    #[test]
    fn text_result_persists_and_returns_preview_tag() {
        with_temp_home_and_cwd(|| {
            let large = "first line\n".repeat(6_000);
            let content = process_tool_result_content(
                "session_abc",
                "toolu_abc",
                "Bash",
                30_000,
                Value::String(large.clone()),
            );
            let text = content.as_str().unwrap();
            assert!(text.starts_with(PERSISTED_OUTPUT_TAG));
            assert!(text.contains("Output too large"));
            assert!(text.contains("Preview (first 2KB):"));
            assert!(text.ends_with(PERSISTED_OUTPUT_CLOSING_TAG));

            let persisted = persisted_path_from_message(text);
            assert_eq!(persisted.file_name().unwrap(), "toolu_abc.txt");
            assert_eq!(std::fs::read_to_string(persisted).unwrap(), large);
        });
    }

    #[test]
    fn json_text_blocks_persist_as_json() {
        with_temp_home_and_cwd(|| {
            let block_text = "x".repeat(60_000);
            let content = process_tool_result_content(
                "session_json",
                "toolu_json",
                "MCPTool",
                100_000,
                json!([{ "type": "text", "text": block_text }]),
            );
            let text = content.as_str().unwrap();
            assert!(text.starts_with(PERSISTED_OUTPUT_TAG));
            let persisted = persisted_path_from_message(text);
            assert_eq!(persisted.file_name().unwrap(), "toolu_json.json");
            let persisted_text = std::fs::read_to_string(persisted).unwrap();
            assert!(persisted_text.contains("\"type\": \"text\""));
        });
    }

    #[test]
    fn image_content_is_not_persisted() {
        let content = json!([{ "type": "image", "source": { "type": "base64", "data": "abc" } }]);
        assert_eq!(
            process_tool_result_content("session", "toolu_img", "Tool", 1, content.clone()),
            content
        );
    }

    #[test]
    fn infinite_tool_cap_opts_out() {
        let content = Value::String("x".repeat(DEFAULT_MAX_RESULT_SIZE_CHARS + 1));
        assert_eq!(
            process_tool_result_content(
                "session",
                "toolu_read",
                "Read",
                usize::MAX,
                content.clone()
            ),
            content
        );
    }

    #[test]
    fn aggregate_budget_persists_largest_fresh_results() {
        with_temp_home_and_cwd(|| {
            let messages = vec![
                json!({"role": "assistant", "content": [
                    {"type": "tool_use", "id": "toolu_a", "name": "Tool", "input": {}},
                    {"type": "tool_use", "id": "toolu_b", "name": "Tool", "input": {}}
                ]}),
                json!({"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "toolu_a", "content": "a".repeat(120_000)},
                    {"type": "tool_result", "tool_use_id": "toolu_b", "content": "b".repeat(120_000)}
                ]}),
            ];
            let mut state = ContentReplacementState::default();
            let out =
                apply_tool_result_budget("session_budget", &messages, &mut state, &HashSet::new());
            let content = out.messages[1]["content"].as_array().unwrap();
            let replaced = content
                .iter()
                .filter(|block| {
                    block["content"]
                        .as_str()
                        .map(|text| text.starts_with(PERSISTED_OUTPUT_TAG))
                        .unwrap_or(false)
                })
                .count();
            assert_eq!(replaced, 1);
            assert_eq!(out.newly_replaced.len(), 1);
            assert_eq!(state.seen_ids.len(), 2);
            assert_eq!(state.replacements.len(), 1);
        });
    }

    #[test]
    fn aggregate_budget_reapplies_existing_replacements_byte_identically() {
        with_temp_home_and_cwd(|| {
            let messages = vec![
                json!({"role": "assistant", "content": [
                    {"type": "tool_use", "id": "toolu_a", "name": "Tool", "input": {}},
                    {"type": "tool_use", "id": "toolu_b", "name": "Tool", "input": {}}
                ]}),
                json!({"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "toolu_a", "content": "a".repeat(120_000)},
                    {"type": "tool_result", "tool_use_id": "toolu_b", "content": "b".repeat(120_000)}
                ]}),
            ];
            let mut state = ContentReplacementState::default();
            let first =
                apply_tool_result_budget("session_reapply", &messages, &mut state, &HashSet::new());
            let second =
                apply_tool_result_budget("session_reapply", &messages, &mut state, &HashSet::new());
            assert_eq!(first.messages, second.messages);
            assert_eq!(second.newly_replaced, Vec::new());
        });
    }

    #[test]
    fn aggregate_budget_skips_infinite_cap_tool_ids() {
        with_temp_home_and_cwd(|| {
            let messages = vec![json!({"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "toolu_read", "content": "r".repeat(250_000)}
            ]})];
            let mut state = ContentReplacementState::default();
            let mut skip = HashSet::new();
            skip.insert("toolu_read".to_string());
            let out = apply_tool_result_budget("session_skip", &messages, &mut state, &skip);
            assert_eq!(out.messages, messages);
            assert!(out.newly_replaced.is_empty());
            assert!(state.seen_ids.contains("toolu_read"));
            assert!(state.replacements.is_empty());
        });
    }
}
