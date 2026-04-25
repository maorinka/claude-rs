use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::secondary_model;
use claude_core::types::events::ToolResultData;

/// Verbatim port of TS WebSearchTool/prompt.ts `getWebSearchPrompt()`.
/// Tool description ported from TS, including behavioural guidance
/// (mandatory "Sources:" section, current-year search queries, US-only note).
///
/// TS interpolates `${currentMonthYear}` computed at call time; the
/// literal month shifts daily, so the Rust port keeps the "current
/// year" guidance generic — callers that want month-year
/// specificity should format it in at splice time.
pub const WEB_SEARCH_PROMPT: &str = include_str!("prompts/web_search.md");

/// WebSearchTool is a client-visible tool backed by an Anthropic server tool.
///
/// The model sees and calls `WebSearch`. The executor then makes a nested
/// request containing the server-side `web_search_20250305` schema and maps
/// `web_search_tool_result` blocks into a normal tool result, matching TS.
pub struct WebSearchTool;

#[async_trait]
impl ToolExecutor for WebSearchTool {
    fn name(&self) -> &str {
        "WebSearch"
    }

    fn description(&self) -> String {
        WEB_SEARCH_PROMPT.to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "minLength": 2,
                    "description": "The search query to use"
                },
                "allowed_domains": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Only include search results from these domains"
                },
                "blocked_domains": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Never include search results from these domains"
                }
            },
            "required": ["query"],
            "additionalProperties": false,
            "$schema": "https://json-schema.org/draft/2020-12/schema"
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    fn is_read_only(&self, _input: &Value) -> bool {
        true
    }

    async fn call(
        &self,
        input: &Value,
        _ctx: &ToolUseContext,
        cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let Some(query) = input.get("query").and_then(Value::as_str) else {
            return Ok(ToolResultData {
                data: json!("Error: Missing query"),
                is_error: true,
            });
        };
        if query.is_empty() {
            return Ok(ToolResultData {
                data: json!("Error: Missing query"),
                is_error: true,
            });
        }

        let allowed_domains = string_array(input.get("allowed_domains"));
        let blocked_domains = string_array(input.get("blocked_domains"));
        if allowed_domains.as_ref().is_some_and(|v| !v.is_empty())
            && blocked_domains.as_ref().is_some_and(|v| !v.is_empty())
        {
            return Ok(ToolResultData {
                data: json!(
                    "Error: Cannot specify both allowed_domains and blocked_domains in the same request"
                ),
                is_error: true,
            });
        }

        let Some(model) = secondary_model::get_global() else {
            return Ok(ToolResultData {
                data: json!(
                    "Error: WebSearch is unavailable because no secondary model is configured"
                ),
                is_error: true,
            });
        };

        let result = model
            .web_search(query, allowed_domains, blocked_domains, cancel)
            .await?;
        Ok(ToolResultData {
            data: json!(result),
            is_error: false,
        })
    }
}

fn string_array(value: Option<&Value>) -> Option<Vec<String>> {
    value.and_then(Value::as_array).map(|items| {
        items
            .iter()
            .filter_map(Value::as_str)
            .map(ToOwned::to_owned)
            .collect()
    })
}

impl WebSearchTool {
    /// Returns the server tool definition that should be included in the API
    /// request's `tools` array. This matches the TS `makeToolSchema()`.
    pub fn server_tool_definition() -> Value {
        json!({
            "type": "web_search_20250305",
            "name": "web_search",
            "max_uses": 8
        })
    }

    /// Returns a server tool definition with domain restrictions.
    pub fn server_tool_definition_with_domains(
        allowed_domains: Option<&[String]>,
        blocked_domains: Option<&[String]>,
    ) -> Value {
        let mut def = json!({
            "type": "web_search_20250305",
            "name": "web_search",
            "max_uses": 8
        });

        if let Some(allowed) = allowed_domains {
            if !allowed.is_empty() {
                def["allowed_domains"] = json!(allowed);
            }
        }
        if let Some(blocked) = blocked_domains {
            if !blocked.is_empty() {
                def["blocked_domains"] = json!(blocked);
            }
        }

        def
    }

    /// Check if web search should be enabled for the current API provider.
    /// In the TS source, this checks for firstParty, Vertex (claude-4.0+ models),
    /// and Foundry providers. We default to true for first-party API usage.
    pub fn is_supported() -> bool {
        // In the Rust implementation, web search is available when using the
        // Anthropic API directly (the default provider). Provider detection
        // can be expanded when Vertex/Foundry support is added.
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_tool_definition() {
        let def = WebSearchTool::server_tool_definition();
        assert_eq!(def["type"], "web_search_20250305");
        assert_eq!(def["name"], "web_search");
        assert_eq!(def["max_uses"], 8);
    }

    #[test]
    fn test_server_tool_definition_with_allowed_domains() {
        let allowed = vec!["example.com".to_string(), "docs.rs".to_string()];
        let def = WebSearchTool::server_tool_definition_with_domains(Some(&allowed), None);
        assert_eq!(def["type"], "web_search_20250305");
        assert_eq!(def["allowed_domains"][0], "example.com");
        assert_eq!(def["allowed_domains"][1], "docs.rs");
        assert!(def.get("blocked_domains").is_none());
    }

    #[test]
    fn test_server_tool_definition_with_blocked_domains() {
        let blocked = vec!["spam.com".to_string()];
        let def = WebSearchTool::server_tool_definition_with_domains(None, Some(&blocked));
        assert_eq!(def["blocked_domains"][0], "spam.com");
        assert!(def.get("allowed_domains").is_none());
    }

    #[test]
    fn test_is_supported() {
        assert!(WebSearchTool::is_supported());
    }

    #[test]
    fn test_tool_executor_schema_name_matches_ts() {
        let tool = WebSearchTool;
        assert_eq!(tool.name(), "WebSearch");
        assert_eq!(tool.input_schema()["required"][0], "query");
    }

    #[test]
    fn test_empty_domain_lists_omitted() {
        let def = WebSearchTool::server_tool_definition_with_domains(Some(&[]), Some(&[]));
        // Empty arrays should not be added
        assert!(def.get("allowed_domains").is_none());
        assert!(def.get("blocked_domains").is_none());
    }
}
