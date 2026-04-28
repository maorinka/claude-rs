use anyhow::{Context, Result};
use async_trait::async_trait;
use base64::Engine;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::mcp::manager::McpManager;
use claude_core::types::events::ToolResultData;

const TOOL_RESULTS_SUBDIR: &str = "tool-results";

// ─── ListMcpResourcesTool ────────────────────────────────────────────────────

/// Verbatim port of TS ListMcpResourcesTool/prompt.ts `PROMPT`
/// (the detailed parameter documentation — distinct from TS
/// `DESCRIPTION` which is the short blurb). Rust port surfaces
/// the detailed variant to the model.
pub const LIST_MCP_RESOURCES_PROMPT: &str = include_str!("prompts/list_mcp_resources.md");

fn sanitize_session_path(name: &str) -> String {
    name.chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect()
}

fn tool_results_dir(ctx: &ToolUseContext) -> Result<PathBuf> {
    let session_id = ctx
        .options
        .session_id
        .as_deref()
        .unwrap_or_else(|| claude_core::api::client::get_session_id().as_str());
    let home = dirs::home_dir().context("could not determine home directory")?;
    Ok(home
        .join(".claude")
        .join("projects")
        .join(sanitize_session_path(
            &ctx.working_directory.display().to_string(),
        ))
        .join(session_id)
        .join(TOOL_RESULTS_SUBDIR))
}

async fn persist_binary_content(
    bytes: &[u8],
    mime_type: Option<&str>,
    persist_id: &str,
    ctx: &ToolUseContext,
) -> Result<(PathBuf, usize)> {
    let dir = tool_results_dir(ctx)?;
    tokio::fs::create_dir_all(&dir).await?;
    let ext = claude_core::mcp::output_storage::extension_for_mime_type(mime_type);
    let filepath = dir.join(format!("{persist_id}.{ext}"));
    tokio::fs::write(&filepath, bytes).await?;
    Ok((filepath, bytes.len()))
}

async fn normalize_read_resource_result(
    data: Value,
    server_name: &str,
    ctx: &ToolUseContext,
) -> Value {
    let Some(contents) = data.get("contents").and_then(|value| value.as_array()) else {
        return data;
    };

    let mut normalized = Vec::with_capacity(contents.len());
    for (index, content) in contents.iter().enumerate() {
        let uri = content.get("uri").cloned().unwrap_or(Value::Null);
        let mime_type = content
            .get("mimeType")
            .and_then(|value| value.as_str())
            .map(str::to_string);

        if let Some(text) = content.get("text").and_then(|value| value.as_str()) {
            normalized.push(json!({
                "uri": uri,
                "mimeType": mime_type,
                "text": text,
            }));
            continue;
        }

        let Some(blob) = content.get("blob").and_then(|value| value.as_str()) else {
            normalized.push(json!({
                "uri": uri,
                "mimeType": mime_type,
            }));
            continue;
        };

        let persist_id = format!(
            "mcp-resource-{}-{}-{}",
            chrono::Utc::now().timestamp_millis(),
            index,
            uuid::Uuid::new_v4()
                .simple()
                .to_string()
                .chars()
                .take(6)
                .collect::<String>()
        );
        match base64::engine::general_purpose::STANDARD.decode(blob) {
            Ok(bytes) => {
                match persist_binary_content(&bytes, mime_type.as_deref(), &persist_id, ctx).await {
                    Ok((filepath, size)) => {
                        let source = format!(
                            "[Resource from {} at {}] ",
                            server_name,
                            uri.as_str().unwrap_or_default()
                        );
                        let filepath_string = filepath.display().to_string();
                        let text = claude_core::mcp::output_storage::get_binary_blob_saved_message(
                            &filepath_string,
                            mime_type.as_deref(),
                            size,
                            &source,
                        );
                        normalized.push(json!({
                            "uri": uri,
                            "mimeType": mime_type,
                            "blobSavedTo": filepath_string,
                            "text": text,
                        }));
                    }
                    Err(error) => {
                        normalized.push(json!({
                            "uri": uri,
                            "mimeType": mime_type,
                            "text": format!("Binary content could not be saved to disk: {error}"),
                        }));
                    }
                }
            }
            Err(error) => {
                normalized.push(json!({
                    "uri": uri,
                    "mimeType": mime_type,
                    "text": format!("Binary content could not be saved to disk: {error}"),
                }));
            }
        }
    }

    json!({ "contents": normalized })
}

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
        ctx: &ToolUseContext,
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
                Ok(data) => {
                    let data = normalize_read_resource_result(data, server, ctx).await;
                    Ok(ToolResultData {
                        data,
                        is_error: false,
                    })
                }
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

    #[test]
    fn session_path_sanitizer_matches_ts_shape() {
        assert_eq!(
            sanitize_session_path("/Users/alice/work/claude-rs"),
            "-Users-alice-work-claude-rs"
        );
    }

    #[tokio::test]
    async fn read_resource_blob_is_not_inlined_when_persist_fails() {
        let ctx = make_ctx();
        let data = json!({
            "contents": [{
                "uri": "file:///tmp/blob.bin",
                "mimeType": "application/octet-stream",
                "blob": "not-valid-base64"
            }]
        });

        let normalized = normalize_read_resource_result(data, "server", &ctx).await;
        let content = &normalized["contents"][0];
        assert_eq!(content["uri"], "file:///tmp/blob.bin");
        assert_eq!(content["mimeType"], "application/octet-stream");
        assert!(content.get("blob").is_none());
        assert!(content["text"]
            .as_str()
            .unwrap()
            .starts_with("Binary content could not be saved to disk:"));
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
