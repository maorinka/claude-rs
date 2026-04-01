use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

pub struct LSPTool;

#[async_trait]
impl ToolExecutor for LSPTool {
    fn name(&self) -> &str {
        "LSP"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["diagnostics", "hover", "definition", "references"],
                    "description": "The LSP action to perform."
                },
                "file_path": {
                    "type": "string",
                    "description": "The absolute path to the file."
                },
                "line": {
                    "type": "integer",
                    "description": "The 1-based line number (required for hover, definition, references)."
                },
                "character": {
                    "type": "integer",
                    "description": "The 1-based character offset (required for hover, definition, references)."
                }
            },
            "required": ["action", "file_path"]
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
        let action = input
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let file_path = input
            .get("file_path")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        match action {
            "diagnostics" => {
                // Use the LSP manager to get diagnostics for the file.
                // For now, create a manager, attempt to get diagnostics.
                // In a full integration the manager would be shared via context.
                let manager = claude_core::lsp::manager::LspManager::new();
                let diagnostics = manager.get_diagnostics(file_path).await?;

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

                Ok(ToolResultData {
                    data: json!({
                        "operation": "diagnostics",
                        "filePath": file_path,
                        "diagnostics": diag_values,
                        "resultCount": diag_values.len()
                    }),
                    is_error: false,
                })
            }
            "hover" => {
                let line = input
                    .get("line")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1);
                let character = input
                    .get("character")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1);

                Ok(ToolResultData {
                    data: json!({
                        "operation": "hover",
                        "filePath": file_path,
                        "line": line,
                        "character": character,
                        "result": format!(
                            "Hover information at {}:{}:{} - LSP hover requires a running language server. \
                             Register a server for this file type to enable hover.",
                            file_path, line, character
                        )
                    }),
                    is_error: false,
                })
            }
            "definition" => {
                let line = input
                    .get("line")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1);
                let character = input
                    .get("character")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1);

                Ok(ToolResultData {
                    data: json!({
                        "operation": "definition",
                        "filePath": file_path,
                        "line": line,
                        "character": character,
                        "result": format!(
                            "Go-to-definition at {}:{}:{} - LSP definition requires a running language server. \
                             Register a server for this file type to enable go-to-definition.",
                            file_path, line, character
                        )
                    }),
                    is_error: false,
                })
            }
            "references" => {
                let line = input
                    .get("line")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1);
                let character = input
                    .get("character")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1);

                Ok(ToolResultData {
                    data: json!({
                        "operation": "references",
                        "filePath": file_path,
                        "line": line,
                        "character": character,
                        "result": format!(
                            "Find references at {}:{}:{} - LSP references requires a running language server. \
                             Register a server for this file type to enable find-references.",
                            file_path, line, character
                        )
                    }),
                    is_error: false,
                })
            }
            _ => Ok(ToolResultData {
                data: json!({
                    "error": format!("Unknown LSP action: '{}'. Supported actions: diagnostics, hover, definition, references", action)
                }),
                is_error: true,
            }),
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
        }
    }

    #[tokio::test]
    async fn test_diagnostics_action() {
        let tool = LSPTool;
        let input = json!({
            "action": "diagnostics",
            "file_path": "/nonexistent/file.rs"
        });
        let result = tool
            .call(&input, &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();

        assert!(!result.is_error);
        assert_eq!(result.data["operation"], "diagnostics");
        assert!(result.data["diagnostics"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_hover_action() {
        let tool = LSPTool;
        let input = json!({
            "action": "hover",
            "file_path": "/some/file.rs",
            "line": 10,
            "character": 5
        });
        let result = tool
            .call(&input, &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();

        assert!(!result.is_error);
        assert_eq!(result.data["operation"], "hover");
        assert!(result.data["result"].as_str().unwrap().contains("hover"));
    }

    #[tokio::test]
    async fn test_definition_action() {
        let tool = LSPTool;
        let input = json!({
            "action": "definition",
            "file_path": "/some/file.rs",
            "line": 5,
            "character": 10
        });
        let result = tool
            .call(&input, &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();

        assert!(!result.is_error);
        assert_eq!(result.data["operation"], "definition");
        assert!(result.data["result"].as_str().unwrap().contains("definition"));
    }

    #[tokio::test]
    async fn test_references_action() {
        let tool = LSPTool;
        let input = json!({
            "action": "references",
            "file_path": "/some/file.rs",
            "line": 5,
            "character": 10
        });
        let result = tool
            .call(&input, &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();

        assert!(!result.is_error);
        assert_eq!(result.data["operation"], "references");
        assert!(result.data["result"].as_str().unwrap().contains("references"));
    }

    #[tokio::test]
    async fn test_unknown_action() {
        let tool = LSPTool;
        let input = json!({
            "action": "unknown_action",
            "file_path": "/some/file.rs"
        });
        let result = tool
            .call(&input, &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.data["error"]
            .as_str()
            .unwrap()
            .contains("Unknown LSP action"));
    }

    #[test]
    fn test_lsp_tool_properties() {
        let tool = LSPTool;
        assert_eq!(tool.name(), "LSP");
        assert!(tool.is_read_only(&json!({})));
        assert!(tool.is_concurrency_safe(&json!({})));
    }

    #[test]
    fn test_input_schema_has_actions() {
        let tool = LSPTool;
        let schema = tool.input_schema();
        let action_enum = &schema["properties"]["action"]["enum"];
        let actions: Vec<&str> = action_enum
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(actions.contains(&"diagnostics"));
        assert!(actions.contains(&"hover"));
        assert!(actions.contains(&"definition"));
        assert!(actions.contains(&"references"));
    }
}
