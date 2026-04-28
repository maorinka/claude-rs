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
        && !is_effective_first_party_anthropic_base_url()
    {
        return false;
    }

    true
}

fn is_effective_first_party_anthropic_base_url() -> bool {
    if let Ok(proxy_base) = std::env::var("CLAUDE_DEBUG_PROXY_BASE") {
        if !proxy_base.is_empty() {
            return is_first_party_anthropic_url(&proxy_base);
        }
    }
    claude_core::privacy_level::is_first_party_anthropic_base_url()
}

fn is_first_party_anthropic_url(raw: &str) -> bool {
    claude_core::privacy_level::is_first_party_anthropic_url(raw)
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
struct ToolSearchEntry {
    name: String,
    description: String,
    deferred: bool,
}

#[derive(Clone, Debug)]
pub struct ToolSearchTool {
    tools: Vec<ToolSearchEntry>,
}

impl ToolSearchTool {
    pub fn new(tools: Vec<(String, String)>) -> Self {
        Self {
            tools: tools
                .into_iter()
                .map(|(name, description)| ToolSearchEntry {
                    deferred: is_deferred_tool_name(&name),
                    name,
                    description,
                })
                .collect(),
        }
    }

    fn with_entries(tools: Vec<ToolSearchEntry>) -> Self {
        Self { tools }
    }
}

impl Default for ToolSearchTool {
    fn default() -> Self {
        Self {
            tools: FALLBACK_TOOLS
                .iter()
                .map(|(name, desc)| ToolSearchEntry {
                    name: (*name).to_string(),
                    description: (*desc).to_string(),
                    deferred: is_deferred_tool_name(name),
                })
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
        let total_deferred_tools = self.tools.iter().filter(|tool| tool.deferred).count();

        if let Some(names) = query.strip_prefix_case_insensitive("select:") {
            let selected: Vec<&str> = names
                .split(',')
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .collect();
            let mut matches: Vec<String> = Vec::new();
            for selected_name in selected {
                let found = self
                    .tools
                    .iter()
                    .find(|tool| tool.deferred && selected_name.eq_ignore_ascii_case(&tool.name))
                    .or_else(|| {
                        self.tools
                            .iter()
                            .find(|tool| selected_name.eq_ignore_ascii_case(&tool.name))
                    });
                if let Some(tool) = found {
                    if !matches.iter().any(|name| name == &tool.name) {
                        matches.push(tool.name.clone());
                    }
                }
            }

            return Ok(ToolResultData {
                data: json!({
                    "matches": matches,
                    "query": query,
                    "total_deferred_tools": total_deferred_tools,
                }),
                is_error: false,
            });
        }

        if let Some(exact) = self
            .tools
            .iter()
            .find(|tool| tool.deferred && tool.name.eq_ignore_ascii_case(query))
            .or_else(|| {
                self.tools
                    .iter()
                    .find(|tool| tool.name.eq_ignore_ascii_case(query))
            })
        {
            return Ok(ToolResultData {
                data: json!({
                    "matches": [exact.name.clone()],
                    "query": query,
                    "total_deferred_tools": total_deferred_tools,
                }),
                is_error: false,
            });
        }

        if query_lower.starts_with("mcp__") && query_lower.len() > 5 {
            let matches = self
                .tools
                .iter()
                .filter(|tool| tool.deferred && tool.name.to_lowercase().starts_with(&query_lower))
                .take(max_results)
                .map(|tool| tool.name.clone())
                .collect::<Vec<_>>();
            if !matches.is_empty() {
                return Ok(ToolResultData {
                    data: json!({
                        "matches": matches,
                        "query": query,
                        "total_deferred_tools": total_deferred_tools,
                    }),
                    is_error: false,
                });
            }
        }

        let (required_terms, query_terms) = split_query_terms(&query_lower);

        let mut scored = self
            .tools
            .iter()
            .filter(|tool| tool.deferred)
            .filter_map(|tool| {
                let parsed = parse_tool_name(&tool.name);
                let haystack = searchable_text(&tool.name, &tool.description);
                if !required_terms.iter().all(|term| {
                    parsed.parts.iter().any(|part| part.contains(term))
                        || description_has_word(&tool.description, term)
                }) {
                    return None;
                }
                let score = query_terms
                    .iter()
                    .map(|term| score_term(&parsed, &haystack, &tool.description, term))
                    .sum::<usize>();
                (score > 0).then_some((tool.name.clone(), score))
            })
            .collect::<Vec<_>>();
        scored.sort_by(|a, b| b.1.cmp(&a.1));
        let matched: Vec<String> = scored
            .into_iter()
            .take(max_results)
            .map(|(name, _)| name)
            .collect();

        Ok(ToolResultData {
            data: json!({
                "matches": matched,
                "query": query,
                "total_deferred_tools": total_deferred_tools,
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
        .map(|tool| ToolSearchEntry {
            name: tool.name().to_string(),
            description: tool.description(),
            deferred: is_deferred_tool_name(tool.name()),
        })
        .collect();
    registry.register(std::sync::Arc::new(ToolSearchTool::with_entries(tools)));
}

trait StripPrefixCaseInsensitive {
    fn strip_prefix_case_insensitive<'a>(&'a self, prefix: &str) -> Option<&'a str>;
}

impl StripPrefixCaseInsensitive for str {
    fn strip_prefix_case_insensitive<'a>(&'a self, prefix: &str) -> Option<&'a str> {
        self.get(..prefix.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
            .then(|| &self[prefix.len()..])
    }
}

#[derive(Debug)]
struct ParsedToolName {
    parts: Vec<String>,
    full: String,
    is_mcp: bool,
}

fn is_deferred_tool_name(name: &str) -> bool {
    name.starts_with("mcp__")
}

fn parse_tool_name(name: &str) -> ParsedToolName {
    if let Some(rest) = name.strip_prefix("mcp__") {
        let full = rest.replace("__", " ").replace('_', " ").to_lowercase();
        let parts = rest
            .to_lowercase()
            .split("__")
            .flat_map(|part| part.split('_'))
            .filter(|part| !part.is_empty())
            .map(str::to_string)
            .collect();
        return ParsedToolName {
            parts,
            full,
            is_mcp: true,
        };
    }

    let full = camel_to_words(name);
    let parts = full
        .split_whitespace()
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect();
    ParsedToolName {
        parts,
        full,
        is_mcp: false,
    }
}

fn score_term(parsed: &ParsedToolName, haystack: &str, description: &str, term: &str) -> usize {
    let mut score = 0;
    if parsed.parts.iter().any(|part| part == term) {
        score += if parsed.is_mcp { 12 } else { 10 };
    } else if parsed.parts.iter().any(|part| part.contains(term)) {
        score += if parsed.is_mcp { 6 } else { 5 };
    }
    if parsed.full.contains(term) && score == 0 {
        score += 3;
    }
    if description_has_word(description, term) {
        score += 2;
    }
    if score == 0 && haystack.contains(term) {
        score += 1;
    }
    score
}

fn description_has_word(description: &str, term: &str) -> bool {
    description
        .to_lowercase()
        .split(|ch: char| !ch.is_alphanumeric())
        .any(|word| word == term)
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
