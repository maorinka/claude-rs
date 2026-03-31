use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

pub struct NotebookEditTool;

fn error_result(msg: impl Into<String>) -> ToolResultData {
    ToolResultData {
        data: json!({ "error": msg.into() }),
        is_error: true,
    }
}

/// Convert a notebook cell's source field (which may be a string or array of strings)
/// into a plain String.
fn source_to_string(source: &Value) -> String {
    match source {
        Value::String(s) => s.clone(),
        Value::Array(arr) => arr
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

#[async_trait]
impl ToolExecutor for NotebookEditTool {
    fn name(&self) -> &str {
        "NotebookEdit"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "notebook_path": {
                    "type": "string",
                    "description": "Absolute path to the .ipynb notebook file"
                },
                "cell_index": {
                    "type": "integer",
                    "description": "0-based index of the cell to edit"
                },
                "new_source": {
                    "type": "string",
                    "description": "New source content for the cell"
                },
                "cell_type": {
                    "type": "string",
                    "description": "Optional: change cell type (code, markdown, raw)"
                }
            },
            "required": ["notebook_path", "cell_index", "new_source"]
        })
    }

    fn is_destructive(&self, _input: &Value) -> bool {
        true
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }

    fn is_read_only(&self, _input: &Value) -> bool {
        false
    }

    async fn call(
        &self,
        input: &Value,
        _ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let notebook_path = match input["notebook_path"].as_str() {
            Some(p) => p.to_string(),
            None => return Ok(error_result("missing required parameter: notebook_path")),
        };
        let cell_index = match input["cell_index"].as_u64() {
            Some(i) => i as usize,
            None => return Ok(error_result("missing required parameter: cell_index")),
        };
        let new_source = match input["new_source"].as_str() {
            Some(s) => s.to_string(),
            None => return Ok(error_result("missing required parameter: new_source")),
        };
        let new_cell_type = input["cell_type"].as_str().map(|s| s.to_string());

        // Read notebook JSON
        let raw = match tokio::fs::read_to_string(&notebook_path).await {
            Ok(s) => s,
            Err(e) => {
                return Ok(error_result(format!(
                    "cannot read notebook '{}': {}",
                    notebook_path, e
                )));
            }
        };

        let mut notebook: Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => {
                return Ok(error_result(format!(
                    "invalid JSON in notebook '{}': {}",
                    notebook_path, e
                )));
            }
        };

        // Access cells array
        let cells = match notebook["cells"].as_array_mut() {
            Some(c) => c,
            None => {
                return Ok(error_result("notebook does not have a 'cells' array"));
            }
        };

        if cell_index >= cells.len() {
            return Ok(error_result(format!(
                "cell_index {} is out of bounds (notebook has {} cells)",
                cell_index,
                cells.len()
            )));
        }

        let cell = &mut cells[cell_index];

        // Capture previous state
        let previous_source = source_to_string(&cell["source"]);
        let current_cell_type = cell["cell_type"].as_str().unwrap_or("code").to_string();

        // Update source (store as string for simplicity)
        cell["source"] = Value::String(new_source.clone());

        // Update cell type if requested
        let final_cell_type = if let Some(ct) = new_cell_type {
            cell["cell_type"] = Value::String(ct.clone());
            ct
        } else {
            current_cell_type
        };

        // Write back
        let updated_json = match serde_json::to_string_pretty(&notebook) {
            Ok(s) => s,
            Err(e) => return Ok(error_result(format!("failed to serialize notebook: {}", e))),
        };

        if let Err(e) = tokio::fs::write(&notebook_path, updated_json).await {
            return Ok(error_result(format!(
                "failed to write notebook '{}': {}",
                notebook_path, e
            )));
        }

        Ok(ToolResultData {
            data: json!({
                "filePath": notebook_path,
                "cellIndex": cell_index,
                "cellType": final_cell_type,
                "previousSource": previous_source,
            }),
            is_error: false,
        })
    }
}
