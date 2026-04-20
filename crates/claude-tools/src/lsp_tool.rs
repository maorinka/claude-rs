use anyhow::Result;
use async_trait::async_trait;
use once_cell::sync::Lazy;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::lsp::manager::LspManager;
use claude_core::types::events::ToolResultData;

// ---------------------------------------------------------------------------
// Shared LspManager singleton
// ---------------------------------------------------------------------------

/// Process-wide LspManager, lazily initialised.
///
/// Mirrors the TS pattern where `getLspServerManager()` returns a global
/// singleton. The `Arc<RwLock<..>>` allows concurrent reads (diagnostics /
/// hover) and exclusive writes (server start / register).
static LSP_MANAGER: Lazy<Arc<RwLock<LspManager>>> =
    Lazy::new(|| Arc::new(RwLock::new(LspManager::new())));

/// Get a reference to the global LspManager.
pub fn get_lsp_manager() -> Arc<RwLock<LspManager>> {
    LSP_MANAGER.clone()
}

/// Register a language server globally. Called during startup.
pub async fn register_lsp_server(
    language_id: &str,
    command: &str,
    args: &[String],
    extensions: &[String],
) {
    let mut mgr = LSP_MANAGER.write().await;
    mgr.register_server(language_id, command, args, extensions);
}

/// Set the workspace root URI on the global manager.
pub async fn set_lsp_root_uri(root_uri: String) {
    let mut mgr = LSP_MANAGER.write().await;
    mgr.set_root_uri(root_uri);
}

/// Check if any LSP server is registered (used for tool visibility).
pub async fn is_lsp_available() -> bool {
    let mgr = LSP_MANAGER.read().await;
    mgr.registered_count() > 0
}

/// Shut down all LSP servers (called at process exit).
pub async fn shutdown_lsp_servers() {
    let mgr = LSP_MANAGER.read().await;
    mgr.shutdown().await;
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Convert a file path to a file:// URI.
fn path_to_file_uri(file_path: &str) -> String {
    if file_path.starts_with('/') {
        format!("file://{}", file_path)
    } else {
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/"));
        let absolute = cwd.join(file_path);
        format!("file://{}", absolute.display())
    }
}

/// Map TS-style operation names to LSP method strings and build params.
fn operation_to_method_and_params(
    operation: &str,
    file_path: &str,
    line: u64,
    character: u64,
) -> Result<(&'static str, Value)> {
    let uri = path_to_file_uri(file_path);
    // Convert from 1-based (user input) to 0-based (LSP protocol)
    let position = json!({
        "line": line.saturating_sub(1),
        "character": character.saturating_sub(1)
    });

    match operation {
        "goToDefinition" => Ok((
            "textDocument/definition",
            json!({ "textDocument": { "uri": uri }, "position": position }),
        )),
        "findReferences" => Ok((
            "textDocument/references",
            json!({
                "textDocument": { "uri": uri },
                "position": position,
                "context": { "includeDeclaration": true }
            }),
        )),
        "hover" => Ok((
            "textDocument/hover",
            json!({ "textDocument": { "uri": uri }, "position": position }),
        )),
        "documentSymbol" => Ok((
            "textDocument/documentSymbol",
            json!({ "textDocument": { "uri": uri } }),
        )),
        "workspaceSymbol" => Ok(("workspace/symbol", json!({ "query": "" }))),
        "goToImplementation" => Ok((
            "textDocument/implementation",
            json!({ "textDocument": { "uri": uri }, "position": position }),
        )),
        "prepareCallHierarchy" => Ok((
            "textDocument/prepareCallHierarchy",
            json!({ "textDocument": { "uri": uri }, "position": position }),
        )),
        "incomingCalls" => Ok((
            "textDocument/prepareCallHierarchy",
            json!({ "textDocument": { "uri": uri }, "position": position }),
        )),
        "outgoingCalls" => Ok((
            "textDocument/prepareCallHierarchy",
            json!({ "textDocument": { "uri": uri }, "position": position }),
        )),
        "diagnostics" => Ok((
            "textDocument/diagnostic",
            json!({ "textDocument": { "uri": uri } }),
        )),
        _ => Err(anyhow::anyhow!(
            "Unknown LSP operation: '{}'. Supported: goToDefinition, findReferences, \
             hover, documentSymbol, workspaceSymbol, goToImplementation, \
             prepareCallHierarchy, incomingCalls, outgoingCalls, diagnostics",
            operation
        )),
    }
}

// ---------------------------------------------------------------------------
// Tool
// ---------------------------------------------------------------------------

pub struct LSPTool;

#[async_trait]
impl ToolExecutor for LSPTool {
    fn name(&self) -> &str {
        "LSP"
    }

    fn description(&self) -> String {
        "Run Language Server Protocol actions on source files. Supports: \
         goToDefinition, findReferences, hover, documentSymbol, workspaceSymbol, \
         goToImplementation, prepareCallHierarchy, incomingCalls, outgoingCalls, diagnostics. \
         Uses the project's registered language servers for real code intelligence."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": [
                        "goToDefinition",
                        "findReferences",
                        "hover",
                        "documentSymbol",
                        "workspaceSymbol",
                        "goToImplementation",
                        "prepareCallHierarchy",
                        "incomingCalls",
                        "outgoingCalls",
                        "diagnostics"
                    ],
                    "description": "The LSP operation to perform."
                },
                "filePath": {
                    "type": "string",
                    "description": "The absolute or relative path to the file."
                },
                "line": {
                    "type": "integer",
                    "description": "The 1-based line number (required for position-based operations)."
                },
                "character": {
                    "type": "integer",
                    "description": "The 1-based character offset (required for position-based operations)."
                }
            },
            "required": ["operation", "filePath"]
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
        let operation = input
            .get("operation")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let file_path = input.get("filePath").and_then(|v| v.as_str()).unwrap_or("");

        if operation.is_empty() {
            return Ok(ToolResultData {
                data: json!({ "error": "missing required field: operation" }),
                is_error: true,
            });
        }
        if file_path.is_empty() {
            return Ok(ToolResultData {
                data: json!({ "error": "missing required field: filePath" }),
                is_error: true,
            });
        }

        // Resolve absolute path
        let absolute_path = if Path::new(file_path).is_absolute() {
            file_path.to_string()
        } else {
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/"));
            cwd.join(file_path).to_string_lossy().to_string()
        };

        let line = input.get("line").and_then(|v| v.as_u64()).unwrap_or(1);
        let character = input.get("character").and_then(|v| v.as_u64()).unwrap_or(1);

        // Special case: diagnostics go through the manager's get_diagnostics
        if operation == "diagnostics" {
            return self.handle_diagnostics(&absolute_path, file_path).await;
        }

        // Map operation to LSP method and params
        let (method, params) =
            match operation_to_method_and_params(operation, &absolute_path, line, character) {
                Ok(v) => v,
                Err(e) => {
                    return Ok(ToolResultData {
                        data: json!({ "error": e.to_string() }),
                        is_error: true,
                    });
                }
            };

        // Open the file in the LSP server if it exists
        if Path::new(&absolute_path).is_file() {
            match tokio::fs::read_to_string(&absolute_path).await {
                Ok(content) => {
                    let mgr = LSP_MANAGER.read().await;
                    let _ = mgr.open_file(&absolute_path, &content).await;
                }
                Err(e) => {
                    tracing::debug!("Could not read file for LSP didOpen: {}", e);
                }
            }
        }

        // Send the request through the LspManager
        let mgr = LSP_MANAGER.read().await;
        let result = mgr.send_request(&absolute_path, method, params).await;

        match result {
            Ok(Some(value)) => {
                // Handle two-step call hierarchy for incomingCalls/outgoingCalls
                let final_value = if operation == "incomingCalls" || operation == "outgoingCalls" {
                    self.handle_call_hierarchy(&mgr, &absolute_path, operation, &value)
                        .await
                        .unwrap_or(value)
                } else {
                    value
                };

                let result_count = if let Some(arr) = final_value.as_array() {
                    arr.len()
                } else if final_value.is_null() {
                    0
                } else {
                    1
                };

                Ok(ToolResultData {
                    data: json!({
                        "operation": operation,
                        "filePath": file_path,
                        "result": final_value,
                        "resultCount": result_count
                    }),
                    is_error: false,
                })
            }
            Ok(None) => {
                let ext = Path::new(file_path)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("unknown");
                Ok(ToolResultData {
                    data: json!({
                        "operation": operation,
                        "filePath": file_path,
                        "result": format!("No LSP server available for file type: .{}", ext),
                        "resultCount": 0
                    }),
                    is_error: false,
                })
            }
            Err(e) => Ok(ToolResultData {
                data: json!({
                    "operation": operation,
                    "filePath": file_path,
                    "result": format!("Error performing {}: {}", operation, e),
                    "resultCount": 0
                }),
                is_error: false,
            }),
        }
    }
}

impl LSPTool {
    /// Handle diagnostics via the manager's dedicated method.
    async fn handle_diagnostics(
        &self,
        absolute_path: &str,
        display_path: &str,
    ) -> Result<ToolResultData> {
        let mgr = LSP_MANAGER.read().await;
        let diagnostics = mgr.get_diagnostics(absolute_path).await?;

        let diag_values: Vec<Value> = diagnostics
            .iter()
            .map(|d| {
                json!({
                    "range": {
                        "start": { "line": d.range.start.line, "character": d.range.start.character },
                        "end": { "line": d.range.end.line, "character": d.range.end.character }
                    },
                    "severity": d.severity.as_ref().map(|s| s.as_str()),
                    "message": d.message,
                    "source": d.source
                })
            })
            .collect();

        let count = diag_values.len();
        Ok(ToolResultData {
            data: json!({
                "operation": "diagnostics",
                "filePath": display_path,
                "result": diag_values,
                "resultCount": count
            }),
            is_error: false,
        })
    }

    /// Handle the two-step call hierarchy for incomingCalls / outgoingCalls.
    /// Step 1 result (prepareCallHierarchy) gives CallHierarchyItems.
    /// Step 2 requests the actual calls using the first item.
    async fn handle_call_hierarchy(
        &self,
        mgr: &LspManager,
        file_path: &str,
        operation: &str,
        prepare_result: &Value,
    ) -> Option<Value> {
        let items = prepare_result.as_array()?;
        if items.is_empty() {
            return Some(json!([]));
        }

        let call_method = if operation == "incomingCalls" {
            "callHierarchy/incomingCalls"
        } else {
            "callHierarchy/outgoingCalls"
        };

        let params = json!({ "item": items[0] });

        match mgr.send_request(file_path, call_method, params).await {
            Ok(Some(result)) => Some(result),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_ctx() -> ToolUseContext {
        ToolUseContext {
            working_directory: PathBuf::from("/tmp"),
            read_file_state: std::sync::Arc::new(std::sync::Mutex::new(
                crate::registry::ReadFileState::new(),
            )),
            permission_mode: crate::registry::PermissionMode::Default,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn test_diagnostics_with_manager() {
        // With no registered servers, diagnostics returns empty (not fake data)
        let tool = LSPTool;
        let input = json!({
            "operation": "diagnostics",
            "filePath": "/nonexistent/file.rs"
        });
        let result = tool
            .call(&input, &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();

        assert!(!result.is_error);
        assert_eq!(result.data["operation"], "diagnostics");
        assert_eq!(result.data["resultCount"], 0);
    }

    #[tokio::test]
    async fn test_hover_uses_manager() {
        let tool = LSPTool;
        let input = json!({
            "operation": "hover",
            "filePath": "/some/file.rs",
            "line": 10,
            "character": 5
        });
        let result = tool
            .call(&input, &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();

        assert!(!result.is_error);
        assert_eq!(result.data["operation"], "hover");
        // Without a running server, result should indicate no server available
        let result_str = result.data["result"].as_str().unwrap_or("");
        assert!(
            result_str.contains("No LSP server") || result.data["resultCount"] == 0,
            "should indicate no server or have zero results"
        );
    }

    #[tokio::test]
    async fn test_go_to_definition_uses_manager() {
        let tool = LSPTool;
        let input = json!({
            "operation": "goToDefinition",
            "filePath": "/some/file.ts",
            "line": 5,
            "character": 10
        });
        let result = tool
            .call(&input, &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();

        assert!(!result.is_error);
        assert_eq!(result.data["operation"], "goToDefinition");
    }

    #[tokio::test]
    async fn test_unknown_operation_returns_error() {
        let tool = LSPTool;
        let input = json!({
            "operation": "nonExistentOp",
            "filePath": "/some/file.rs"
        });
        let result = tool
            .call(&input, &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.data["error"]
            .as_str()
            .unwrap()
            .contains("Unknown LSP operation"));
    }

    #[tokio::test]
    async fn test_missing_operation_returns_error() {
        let tool = LSPTool;
        let input = json!({
            "filePath": "/some/file.rs"
        });
        let result = tool
            .call(&input, &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.data["error"].as_str().unwrap().contains("operation"));
    }

    #[tokio::test]
    async fn test_missing_file_path_returns_error() {
        let tool = LSPTool;
        let input = json!({
            "operation": "hover"
        });
        let result = tool
            .call(&input, &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.data["error"].as_str().unwrap().contains("filePath"));
    }

    #[test]
    fn test_lsp_tool_properties() {
        let tool = LSPTool;
        assert_eq!(tool.name(), "LSP");
        assert!(tool.is_read_only(&json!({})));
        assert!(tool.is_concurrency_safe(&json!({})));
    }

    #[test]
    fn test_input_schema_has_all_operations() {
        let tool = LSPTool;
        let schema = tool.input_schema();
        let op_enum = &schema["properties"]["operation"]["enum"];
        let ops: Vec<&str> = op_enum
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(ops.contains(&"goToDefinition"));
        assert!(ops.contains(&"findReferences"));
        assert!(ops.contains(&"hover"));
        assert!(ops.contains(&"documentSymbol"));
        assert!(ops.contains(&"workspaceSymbol"));
        assert!(ops.contains(&"goToImplementation"));
        assert!(ops.contains(&"prepareCallHierarchy"));
        assert!(ops.contains(&"incomingCalls"));
        assert!(ops.contains(&"outgoingCalls"));
        assert!(ops.contains(&"diagnostics"));
    }

    #[test]
    fn test_path_to_file_uri() {
        let uri = path_to_file_uri("/home/user/test.rs");
        assert_eq!(uri, "file:///home/user/test.rs");
    }

    #[test]
    fn test_operation_to_method() {
        let (method, _) =
            operation_to_method_and_params("goToDefinition", "/test.rs", 10, 5).unwrap();
        assert_eq!(method, "textDocument/definition");

        let (method, _) =
            operation_to_method_and_params("findReferences", "/test.rs", 10, 5).unwrap();
        assert_eq!(method, "textDocument/references");

        let (method, _) = operation_to_method_and_params("hover", "/test.rs", 10, 5).unwrap();
        assert_eq!(method, "textDocument/hover");

        let result = operation_to_method_and_params("badOp", "/test.rs", 10, 5);
        assert!(result.is_err());
    }
}
