use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

// ─── ListMcpResourcesTool ────────────────────────────────────────────────────

pub struct ListMcpResourcesTool;

#[async_trait]
impl ToolExecutor for ListMcpResourcesTool {
    fn name(&self) -> &str {
        "ListMcpResourcesTool"
    }

    fn description(&self) -> String {
        r#"Lists available resources from configured MCP servers.
Each resource object includes a 'server' field indicating which server it's from.

Usage examples:
- List all resources from all servers: `listMcpResources`
- List resources from a specific server: `listMcpResources({ server: "myserver" })`"#
            .to_string()
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
        let _server = input.get("server").and_then(|v| v.as_str());

        // MCP resource listing requires a connected MCP manager.
        // In the current architecture, MCP resources are served through the
        // McpManager. This tool acts as a stub that returns an empty list
        // until the full MCP resource API is wired up.
        Ok(ToolResultData {
            data: json!({
                "resources": [],
                "message": "No MCP resources found. MCP servers may still provide tools even if they have no resources."
            }),
            is_error: false,
        })
    }
}

// ─── ReadMcpResourceTool ─────────────────────────────────────────────────────

pub struct ReadMcpResourceTool;

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

        // MCP resource reading requires a connected MCP manager.
        // This tool acts as a stub that returns a not-found error
        // until the full MCP resource API is wired up.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ReadFileState;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn make_ctx() -> ToolUseContext {
        ToolUseContext {
            working_directory: PathBuf::from("/tmp"),
            read_file_state: Arc::new(std::sync::Mutex::new(ReadFileState::new())),
        }
    }

    #[tokio::test]
    async fn list_mcp_resources_returns_empty() {
        let tool = ListMcpResourcesTool;
        let input = json!({});
        let ctx = make_ctx();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(!result.is_error);
        assert!(result.data["resources"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn list_mcp_resources_with_server_filter() {
        let tool = ListMcpResourcesTool;
        let input = json!({ "server": "test-server" });
        let ctx = make_ctx();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(!result.is_error);
    }

    #[tokio::test]
    async fn read_mcp_resource_missing_server() {
        let tool = ReadMcpResourceTool;
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
        let tool = ReadMcpResourceTool;
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
        let tool = ReadMcpResourceTool;
        let input = json!({ "server": "my-server", "uri": "file://test.txt" });
        let ctx = make_ctx();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(result.is_error);
        assert!(result.data["error"].as_str().unwrap().contains("my-server"));
    }
}
