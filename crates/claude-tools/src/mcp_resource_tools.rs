use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::mcp::manager::McpManager;
use claude_core::types::events::ToolResultData;

// ─── ListMcpResourcesTool ────────────────────────────────────────────────────

/// Verbatim port of TS ListMcpResourcesTool/prompt.ts `PROMPT`
/// (the detailed parameter documentation — distinct from TS
/// `DESCRIPTION` which is the short blurb). Rust port surfaces
/// the detailed variant to the model.
pub const LIST_MCP_RESOURCES_PROMPT: &str = include_str!("prompts/list_mcp_resources.md");

#[derive(Default)]
pub struct ListMcpResourcesTool {
    manager: Option<Arc<RwLock<McpManager>>>,
}

impl ListMcpResourcesTool {
    pub fn new(manager: Arc<RwLock<McpManager>>) -> Self {
        Self {
            manager: Some(manager),
        }
    }
}

#[async_trait]
impl ToolExecutor for ListMcpResourcesTool {
    fn name(&self) -> &str {
        "ListMcpResourcesTool"
    }

    fn description(&self) -> String {
        LIST_MCP_RESOURCES_PROMPT.to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "server": {
                    "type": "string",
                    "description": "Optional server name to filter resources by"
                }
            }
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
        let server = input.get("server").and_then(|v| v.as_str());

        if let Some(manager) = &self.manager {
            let manager = manager.read().await;
            let mut resources = manager.list_resources().await?;
            if let Some(server) = server {
                resources.retain(|resource| resource.server == server);
            }

            return Ok(ToolResultData {
                data: json!(resources),
                is_error: false,
            });
        }

        Ok(ToolResultData {
            data: json!([]),
            is_error: false,
        })
    }
}

// ─── ReadMcpResourceTool ─────────────────────────────────────────────────────

#[derive(Default)]
pub struct ReadMcpResourceTool {
    manager: Option<Arc<RwLock<McpManager>>>,
}

impl ReadMcpResourceTool {
    pub fn new(manager: Arc<RwLock<McpManager>>) -> Self {
        Self {
            manager: Some(manager),
        }
    }
}

#[async_trait]
impl ToolExecutor for ReadMcpResourceTool {
    fn name(&self) -> &str {
        "ReadMcpResourceTool"
    }

    fn description(&self) -> String {
        r#"Reads a specific resource from an MCP server, identified by server name and resource URI.

Parameters:
- server (required): The name of the MCP server from which to read the resource
- uri (required): The URI of the resource to read"#
            .to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "server": {
                    "type": "string",
                    "description": "The MCP server name"
                },
                "uri": {
                    "type": "string",
                    "description": "The resource URI to read"
                }
            },
            "required": ["server", "uri"]
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
        let server = match input.get("server").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => {
                return Ok(ToolResultData {
                    data: json!({ "error": "missing required field: server" }),
                    is_error: true,
                });
            }
        };

        let uri = match input.get("uri").and_then(|v| v.as_str()) {
            Some(u) => u,
            None => {
                return Ok(ToolResultData {
                    data: json!({ "error": "missing required field: uri" }),
                    is_error: true,
                });
            }
        };

        if let Some(manager) = &self.manager {
            let manager = manager.read().await;
            return match manager.read_resource(server, uri).await {
                Ok(data) => Ok(ToolResultData {
                    data,
                    is_error: false,
                }),
                Err(error) => Ok(ToolResultData {
                    data: json!({ "error": error.to_string() }),
                    is_error: true,
                }),
            };
        }

        Ok(ToolResultData {
            data: json!({
                "error": format!(
                    "Server \"{}\" not found or resource \"{}\" is not available. Ensure the MCP server is connected.",
                    server, uri
                )
            }),
            is_error: true,
        })
    }
}

/// Register manager-backed MCP resource tools.
pub fn register_mcp_resource_tools(
    registry: &mut crate::registry::ToolRegistry,
    manager: Arc<RwLock<McpManager>>,
) {
    registry.register(Arc::new(ListMcpResourcesTool::new(manager.clone())));
    registry.register(Arc::new(ReadMcpResourceTool::new(manager)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ReadFileState;
    use claude_core::mcp::manager::McpManager;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn make_ctx() -> ToolUseContext {
        ToolUseContext::for_test(
            PathBuf::from("/tmp"),
            Arc::new(std::sync::Mutex::new(ReadFileState::new())),
            crate::registry::PermissionMode::Default,
        )
    }

    #[tokio::test]
    async fn list_mcp_resources_returns_empty() {
        let tool = ListMcpResourcesTool::default();
        let input = json!({});
        let ctx = make_ctx();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(!result.is_error);
        assert!(result.data.as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn list_mcp_resources_with_server_filter() {
        let tool = ListMcpResourcesTool::default();
        let input = json!({ "server": "test-server" });
        let ctx = make_ctx();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(!result.is_error);
    }

    #[tokio::test]
    async fn read_mcp_resource_missing_server() {
        let tool = ReadMcpResourceTool::default();
        let input = json!({ "uri": "test://resource" });
        let ctx = make_ctx();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(result.is_error);
        assert!(result.data["error"]
            .as_str()
            .unwrap()
            .contains("missing required field: server"));
    }

    #[tokio::test]
    async fn read_mcp_resource_missing_uri() {
        let tool = ReadMcpResourceTool::default();
        let input = json!({ "server": "test-server" });
        let ctx = make_ctx();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(result.is_error);
        assert!(result.data["error"]
            .as_str()
            .unwrap()
            .contains("missing required field: uri"));
    }

    #[tokio::test]
    async fn read_mcp_resource_stub_error() {
        let tool = ReadMcpResourceTool::default();
        let input = json!({ "server": "my-server", "uri": "file://test.txt" });
        let ctx = make_ctx();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(result.is_error);
        assert!(result.data["error"].as_str().unwrap().contains("my-server"));
    }

    #[tokio::test]
    async fn manager_backed_list_uses_connected_manager() {
        let manager = Arc::new(RwLock::new(McpManager::new()));
        let tool = ListMcpResourcesTool::new(manager);
        let input = json!({});
        let ctx = make_ctx();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(!result.is_error);
        assert!(result.data.as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn manager_backed_read_reports_manager_error() {
        let manager = Arc::new(RwLock::new(McpManager::new()));
        let tool = ReadMcpResourceTool::new(manager);
        let input = json!({ "server": "missing", "uri": "file://test.txt" });
        let ctx = make_ctx();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(result.is_error);
        assert!(result.data["error"]
            .as_str()
            .unwrap()
            .contains("MCP server 'missing' is not connected"));
    }
}
