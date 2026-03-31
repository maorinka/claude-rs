use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

/// Static list of all known tool names and their descriptions.
/// This is kept in sync with `build_default_registry`.
const ALL_TOOLS: &[(&str, &str)] = &[
    ("Bash", "Execute a bash command in a shell"),
    ("Read", "Read the contents of a file"),
    ("Write", "Write content to a file"),
    ("Edit", "Edit a specific portion of a file"),
    ("Grep", "Search for patterns in files using ripgrep"),
    ("Glob", "Find files matching a glob pattern"),
    ("Config", "Get, set, or list Claude configuration settings"),
    ("EnterPlanMode", "Switch to plan mode for describing actions before executing"),
    ("ExitPlanMode", "Return to normal mode after planning"),
    ("AskUser", "Ask the user a question and receive an answer"),
    ("Brief", "Toggle brief mode for more concise output"),
    ("SendMessage", "Send a message to another agent or channel"),
    ("MCP", "Call a tool on a connected MCP server"),
    ("LSP", "Run an LSP action such as diagnostics on a file"),
    ("ToolSearch", "Search registered tools by name or description"),
];

pub struct ToolSearchTool;

#[async_trait]
impl ToolExecutor for ToolSearchTool {
    fn name(&self) -> &str {
        "ToolSearch"
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
            Some(q) => q.to_lowercase(),
            None => {
                return Ok(ToolResultData {
                    data: json!({ "error": "missing required field: query" }),
                    is_error: true,
                });
            }
        };

        let max_results = input["max_results"].as_u64().unwrap_or(5) as usize;

        let matched: Vec<Value> = ALL_TOOLS
            .iter()
            .filter(|(name, desc)| {
                name.to_lowercase().contains(&query) || desc.to_lowercase().contains(&query)
            })
            .take(max_results)
            .map(|(name, desc)| json!({ "name": name, "description": desc }))
            .collect();

        Ok(ToolResultData {
            data: json!({ "tools": matched }),
            is_error: false,
        })
    }
}
