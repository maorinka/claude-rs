use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::debug;

use claude_core::mcp::helpers::mcp_tool_input_to_auto_classifier_input;
use claude_core::mcp::manager::McpManager;
use claude_core::types::events::ToolResultData;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};

/// An MCP tool that delegates to an MCP server via the McpManager.
///
/// Each instance represents a single tool from a connected MCP server.
/// The normalized name (mcp__{server}__{tool}) is used for tool dispatch.
pub struct McpTool {
    /// Normalized tool name: mcp__{server}__{tool}
    name: String,
    /// Original tool name from the MCP server.
    original_name: String,
    /// Server this tool belongs to.
    server_name: String,
    /// Tool description (used for display / tool listing).
    #[allow(dead_code)]
    description: String,
    /// Input schema.
    input_schema: Value,
    /// Reference to the MCP manager for executing tool calls.
    manager: Arc<RwLock<McpManager>>,
}

impl McpTool {
    pub fn new(
        name: String,
        original_name: String,
        server_name: String,
        description: String,
        input_schema: Value,
        manager: Arc<RwLock<McpManager>>,
    ) -> Self {
        Self {
            name,
            original_name,
            server_name,
            description,
            input_schema,
            manager,
        }
    }
}

#[async_trait]
impl ToolExecutor for McpTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn input_schema(&self) -> Value {
        self.input_schema.clone()
    }

    fn description(&self) -> String {
        self.description.clone()
    }

    async fn call(
        &self,
        input: &Value,
        _ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        debug!(
            tool = self.name,
            server = self.server_name,
            original = self.original_name,
            "Calling MCP tool"
        );

        let manager = self.manager.read().await;
        let result = manager.call_tool(&self.name, input.clone()).await;

        match result {
            Ok(mcp_result) => {
                // Convert MCP result to ToolResultData
                let is_error = mcp_result.is_error.unwrap_or(false);

                // Collect text content from the result
                let text_parts: Vec<String> = mcp_result
                    .content
                    .iter()
                    .filter_map(|c| c.text.clone())
                    .collect();

                let output = if text_parts.is_empty() {
                    // If no text, serialize the full result
                    serde_json::to_value(&mcp_result.content)
                        .unwrap_or(Value::String("(empty result)".to_string()))
                } else if text_parts.len() == 1 {
                    Value::String(text_parts.into_iter().next().unwrap())
                } else {
                    Value::String(text_parts.join("\n"))
                };

                Ok(ToolResultData {
                    data: output,
                    is_error,
                })
            }
            Err(e) => Ok(ToolResultData {
                data: Value::String(format!("MCP tool call failed: {}", e)),
                is_error: true,
            }),
        }
    }

    fn is_read_only(&self, _input: &Value) -> bool {
        // MCP tools are unknown in their side effects - assume not read-only
        false
    }

    fn to_auto_classifier_input(&self, input: &Value) -> Option<String> {
        let Some(map) = input.as_object() else {
            return Some(self.name.clone());
        };
        Some(mcp_tool_input_to_auto_classifier_input(map, &self.name))
    }
}

/// Register all MCP tools from the manager into a tool registry.
pub async fn register_mcp_tools(
    registry: &mut crate::registry::ToolRegistry,
    manager: Arc<RwLock<McpManager>>,
) {
    let mgr = manager.read().await;
    let tool_defs = mgr.tool_definitions().await;
    drop(mgr);

    let mut tool_defs = tool_defs;
    tool_defs.sort_by(|a, b| a.name.cmp(&b.name));

    for tool_info in tool_defs {
        let input_schema = tool_info
            .input_schema
            .unwrap_or_else(|| serde_json::json!({"type": "object", "properties": {}}));

        let mcp_tool = McpTool::new(
            tool_info.name,
            tool_info.original_name,
            tool_info.server_name,
            tool_info.description.unwrap_or_default(),
            input_schema,
            manager.clone(),
        );

        registry.register(Arc::new(mcp_tool));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_tool() -> McpTool {
        McpTool::new(
            "mcp__svc__search".to_string(),
            "search".to_string(),
            "svc".to_string(),
            "Search".to_string(),
            json!({"type": "object"}),
            Arc::new(RwLock::new(McpManager::new())),
        )
    }

    #[test]
    fn classifier_input_uses_ts_mcp_projection() {
        let tool = test_tool();
        let input = json!({
            "query": "rust",
            "opts": {"limit": 3},
            "items": [1, 2, 3]
        });
        assert_eq!(
            tool.to_auto_classifier_input(&input).as_deref(),
            Some("query=rust opts=[object Object] items=1,2,3")
        );
    }

    #[test]
    fn classifier_input_empty_mcp_input_uses_tool_name() {
        let tool = test_tool();
        assert_eq!(
            tool.to_auto_classifier_input(&json!({})).as_deref(),
            Some("mcp__svc__search")
        );
    }
}
