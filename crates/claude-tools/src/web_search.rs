use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

/// WebSearchTool is a **server-side** tool.
///
/// Unlike regular client-side tools, web search is handled by Anthropic's API
/// server. The tool definition (`web_search_20250305`) is injected into the
/// request body by `build_request_body()` in `claude-core`, and the API handles
/// search execution internally.
///
/// This struct is kept in the registry so that the tool is visible in listings
/// and schemas, but its `call()` method should never be invoked directly —
/// the API processes `server_tool_use` / `web_search_tool_result` content
/// blocks without a client-side tool_result round-trip.
pub struct WebSearchTool;

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
}

#[async_trait]
impl ToolExecutor for WebSearchTool {
    fn name(&self) -> &str {
        "WebSearch"
    }

    fn description(&self) -> String {
        "Search the web for current information using Anthropic's server-side \
         web search. This tool is handled by the API — the model can invoke it \
         automatically when web_search is included in the request tools."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query"
                },
                "allowed_domains": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional list of domains to restrict results to"
                },
                "blocked_domains": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional list of domains to exclude from results"
                }
            },
            "required": ["query"]
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
        _input: &Value,
        _ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        // Web search is a server-side tool — the API handles it via
        // server_tool_use / web_search_tool_result content blocks.
        // This call() should not be reached in normal operation.
        Ok(ToolResultData {
            data: json!({
                "error": "WebSearch is a server-side tool handled by the Anthropic API. \
                          It should not be called client-side."
            }),
            is_error: true,
        })
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
    fn test_web_search_tool_name() {
        let tool = WebSearchTool;
        assert_eq!(tool.name(), "WebSearch");
    }

    #[test]
    fn test_web_search_is_read_only() {
        let tool = WebSearchTool;
        assert!(tool.is_read_only(&json!({})));
    }

    #[test]
    fn test_web_search_is_concurrency_safe() {
        let tool = WebSearchTool;
        assert!(tool.is_concurrency_safe(&json!({})));
    }

    #[test]
    fn test_web_search_schema_has_query() {
        let tool = WebSearchTool;
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "query"));
        assert!(schema["properties"]["query"].is_object());
        assert!(schema["properties"]["allowed_domains"].is_object());
        assert!(schema["properties"]["blocked_domains"].is_object());
    }
}
