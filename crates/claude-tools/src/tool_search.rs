use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolSearchMode {
    ToolSearch,
    ToolSearchAuto,
    Standard,
}

fn is_truthy_value(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn parse_auto_percentage(value: &str) -> Option<u8> {
    let percent = value.strip_prefix("auto:")?.parse::<i32>().ok()?;
    Some(percent.clamp(0, 100) as u8)
}

pub fn get_tool_search_mode() -> ToolSearchMode {
    if claude_core::errors_util::is_env_truthy("CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS") {
        return ToolSearchMode::Standard;
    }

    let value = std::env::var("ENABLE_TOOL_SEARCH").ok();
    if let Some(value) = value.as_deref() {
        if let Some(percent) = parse_auto_percentage(value) {
            return match percent {
                0 => ToolSearchMode::ToolSearch,
                100 => ToolSearchMode::Standard,
                _ => ToolSearchMode::ToolSearchAuto,
            };
        }
        if value == "auto" || value.starts_with("auto:") {
            return ToolSearchMode::ToolSearchAuto;
        }
        if is_truthy_value(value) {
            return ToolSearchMode::ToolSearch;
        }
    }

    if claude_core::errors_util::is_env_definitely_falsy("ENABLE_TOOL_SEARCH") {
        return ToolSearchMode::Standard;
    }

    ToolSearchMode::ToolSearch
}

pub fn is_tool_search_enabled_optimistic() -> bool {
    if get_tool_search_mode() == ToolSearchMode::Standard {
        return false;
    }

    let explicitly_configured = std::env::var("ENABLE_TOOL_SEARCH")
        .ok()
        .is_some_and(|value| !value.is_empty());
    if !explicitly_configured
        && claude_core::privacy_level::get_api_provider()
            == claude_core::privacy_level::ApiProvider::FirstParty
        && !claude_core::privacy_level::is_first_party_anthropic_base_url()
    {
        return false;
    }

    true
}

/// Fallback list used by direct tests or embedders that construct the tool
/// without a registry snapshot.
const FALLBACK_TOOLS: &[(&str, &str)] = &[
    ("Bash", "Execute a bash command in a shell"),
    ("Read", "Read the contents of a file"),
    ("Write", "Write content to a file"),
    ("Edit", "Edit a specific portion of a file"),
    ("Grep", "Search for patterns in files using ripgrep"),
    ("Glob", "Find files matching a glob pattern"),
    ("WebSearch", "Search the web for current information"),
    ("Config", "Get, set, or list Claude configuration settings"),
    (
        "EnterPlanMode",
        "Switch to plan mode for describing actions before executing",
    ),
    ("ExitPlanMode", "Return to normal mode after planning"),
    (
        "AskUserQuestion",
        "Ask the user a question and receive an answer",
    ),
    ("Brief", "Toggle brief mode for more concise output"),
    ("SendMessage", "Send a message to another agent or channel"),
    ("MCP", "Call a tool on a connected MCP server"),
    ("LSP", "Run an LSP action such as diagnostics on a file"),
    (
        "ToolSearch",
        "Search registered tools by name or description",
    ),
];

#[derive(Clone, Debug)]
pub struct ToolSearchTool {
    tools: Vec<(String, String)>,
}

impl ToolSearchTool {
    pub fn new(tools: Vec<(String, String)>) -> Self {
        Self { tools }
    }
}

impl Default for ToolSearchTool {
    fn default() -> Self {
        Self {
            tools: FALLBACK_TOOLS
                .iter()
                .map(|(name, desc)| ((*name).to_string(), (*desc).to_string()))
                .collect(),
        }
    }
}

#[async_trait]
impl ToolExecutor for ToolSearchTool {
    fn name(&self) -> &str {
        "ToolSearch"
    }

    fn description(&self) -> String {
        r#"Fetches full schema definitions for deferred tools so they can be called.

Deferred tools appear by name in <system-reminder> messages. Until fetched, only the name is known -- there is no parameter schema, so the tool cannot be invoked. This tool takes a query, matches it against the deferred tool list, and returns the matched tools' complete JSONSchema definitions inside a <functions> block. Once a tool's schema appears in that result, it is callable exactly like any tool defined at the top of the prompt.

Query forms:
- "select:Read,Edit,Grep" -- fetch these exact tools by name
- "notebook jupyter" -- keyword search, up to max_results best matches
- "+slack send" -- require "slack" in the name, rank by remaining terms"#.to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search term to match against tool names and descriptions."
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of results to return. Defaults to 5."
                }
            },
            "required": ["query"]
        })
    }

    fn is_read_only(&self, _input: &Value) -> bool {
        true
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    async fn call(
        &self,
        input: &Value,
        _ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let query = match input["query"].as_str() {
            Some(q) => q.trim(),
            None => {
                return Ok(ToolResultData {
                    data: json!({ "error": "missing required field: query" }),
                    is_error: true,
                });
            }
        };

        let max_results = input["max_results"].as_u64().unwrap_or(5) as usize;
        let query_lower = query.to_lowercase();

        if let Some(names) = query_lower.strip_prefix("select:") {
            let selected: Vec<&str> = names
                .split(',')
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .collect();
            let matched: Vec<Value> = self
                .tools
                .iter()
                .filter(|(name, _)| {
                    selected
                        .iter()
                        .any(|selected_name| selected_name.eq_ignore_ascii_case(name))
                })
                .take(max_results)
                .map(|(name, desc)| json!({ "name": name, "description": desc }))
                .collect();

            return Ok(ToolResultData {
                data: json!({
                    "matches": matched.iter().filter_map(|tool| tool["name"].as_str()).collect::<Vec<_>>(),
                    "tools": matched,
                }),
                is_error: false,
            });
        }

        let (required_name_terms, query_terms) = split_query_terms(&query_lower);

        let matched: Vec<Value> = self
            .tools
            .iter()
            .filter(|(name, desc)| {
                let haystack = searchable_text(name, desc);
                required_name_terms.iter().all(|term| {
                    name.to_lowercase().contains(term) || camel_to_words(name).contains(term)
                }) && query_terms.iter().all(|term| haystack.contains(term))
            })
            .take(max_results)
            .map(|(name, desc)| json!({ "name": name, "description": desc }))
            .collect();

        Ok(ToolResultData {
            data: json!({
                "matches": matched.iter().filter_map(|tool| tool["name"].as_str()).collect::<Vec<_>>(),
                "tools": matched,
            }),
            is_error: false,
        })
    }
}

pub fn register_tool_search_snapshot(registry: &mut crate::registry::ToolRegistry) {
    if !is_tool_search_enabled_optimistic() {
        return;
    }

    let tools = registry
        .all()
        .iter()
        .map(|tool| (tool.name().to_string(), tool.description()))
        .collect();
    registry.register(std::sync::Arc::new(ToolSearchTool::new(tools)));
}

fn split_query_terms(query: &str) -> (Vec<String>, Vec<String>) {
    query
        .split_whitespace()
        .fold((Vec::new(), Vec::new()), |mut acc, term| {
            if let Some(required) = term.strip_prefix('+') {
                if !required.is_empty() {
                    acc.0.push(required.to_string());
                }
            } else if !term.is_empty() {
                acc.1.push(term.to_string());
            }
            acc
        })
}

fn searchable_text(name: &str, description: &str) -> String {
    format!(
        "{} {} {}",
        name.to_lowercase(),
        camel_to_words(name),
        description.to_lowercase()
    )
}

fn camel_to_words(name: &str) -> String {
    let mut words = String::with_capacity(name.len() + 4);
    for (index, ch) in name.chars().enumerate() {
        if index > 0 && ch.is_uppercase() {
            words.push(' ');
        } else if ch == '_' || ch == '-' {
            words.push(' ');
            continue;
        }
        for lower in ch.to_lowercase() {
            words.push(lower);
        }
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_tool_search_env(value: Option<&str>, f: impl FnOnce()) {
        let _guard = ENV_LOCK.lock().unwrap();
        let old_enable = std::env::var("ENABLE_TOOL_SEARCH").ok();
        let old_disable = std::env::var("CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS").ok();

        match value {
            Some(value) => std::env::set_var("ENABLE_TOOL_SEARCH", value),
            None => std::env::remove_var("ENABLE_TOOL_SEARCH"),
        }
        std::env::remove_var("CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS");

        f();

        match old_enable {
            Some(value) => std::env::set_var("ENABLE_TOOL_SEARCH", value),
            None => std::env::remove_var("ENABLE_TOOL_SEARCH"),
        }
        match old_disable {
            Some(value) => std::env::set_var("CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS", value),
            None => std::env::remove_var("CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS"),
        }
    }

    #[test]
    fn tool_search_mode_matches_ts_auto_variants() {
        with_tool_search_env(Some("auto"), || {
            assert_eq!(get_tool_search_mode(), ToolSearchMode::ToolSearchAuto);
        });
        with_tool_search_env(Some("auto:1"), || {
            assert_eq!(get_tool_search_mode(), ToolSearchMode::ToolSearchAuto);
        });
        with_tool_search_env(Some("auto:99"), || {
            assert_eq!(get_tool_search_mode(), ToolSearchMode::ToolSearchAuto);
        });
        with_tool_search_env(Some("auto:not-a-number"), || {
            assert_eq!(get_tool_search_mode(), ToolSearchMode::ToolSearchAuto);
        });
    }

    #[test]
    fn tool_search_mode_matches_ts_auto_edges() {
        with_tool_search_env(Some("auto:0"), || {
            assert_eq!(get_tool_search_mode(), ToolSearchMode::ToolSearch);
        });
        with_tool_search_env(Some("auto:-5"), || {
            assert_eq!(get_tool_search_mode(), ToolSearchMode::ToolSearch);
        });
        with_tool_search_env(Some("auto:100"), || {
            assert_eq!(get_tool_search_mode(), ToolSearchMode::Standard);
        });
        with_tool_search_env(Some("auto:500"), || {
            assert_eq!(get_tool_search_mode(), ToolSearchMode::Standard);
        });
    }
}
