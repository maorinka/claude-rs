use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

pub struct WebSearchTool;

#[async_trait]
impl ToolExecutor for WebSearchTool {
    fn name(&self) -> &str {
        "WebSearch"
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
        input: &Value,
        _ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let query = match input["query"].as_str() {
            Some(q) => q.to_string(),
            None => {
                return Ok(ToolResultData {
                    data: json!({ "error": "missing required parameter: query" }),
                    is_error: true,
                });
            }
        };

        // Stub implementation: web search requires server-side support
        Ok(ToolResultData {
            data: json!({
                "query": query,
                "results": [],
                "durationSeconds": 0.0,
                "message": "Web search is not yet available in this environment. \
                            It requires server-side search API support."
            }),
            is_error: false,
        })
    }
}
