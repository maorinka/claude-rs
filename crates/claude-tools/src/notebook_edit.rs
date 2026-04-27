use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

pub struct NotebookEditTool;

fn error_result(msg: impl Into<String>) -> ToolResultData {
    error_result_with_fields(msg, "", "", "code", "python", "replace", None, "", "")
}

fn error_result_with_fields(
    msg: impl Into<String>,
    notebook_path: &str,
    new_source: &str,
    cell_type: &str,
    language: &str,
    edit_mode: &str,
    cell_id: Option<&str>,
    original_file: &str,
    updated_file: &str,
) -> ToolResultData {
    ToolResultData {
        data: json!({
            "new_source": new_source,
            "cell_type": cell_type,
            "language": language,
            "edit_mode": edit_mode,
            "error": msg.into(),
            "cell_id": cell_id,
            "notebook_path": notebook_path,
            "original_file": original_file,
            "updated_file": updated_file,
        }),
        is_error: true,
    }
}

fn parse_cell_id(cell_id: &str) -> Option<usize> {
    cell_id
        .strip_prefix("cell-")
        .and_then(|value| value.parse::<usize>().ok())
}

fn resolve_notebook_path(path: &str, cwd: &Path) -> String {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        path.display().to_string()
    } else {
        cwd.join(path).display().to_string()
    }
}

#[async_trait]
impl ToolExecutor for NotebookEditTool {
    fn name(&self) -> &str {
        "NotebookEdit"
    }

    fn description(&self) -> String {
        "Replace the contents of a specific cell in a Jupyter notebook (.ipynb file) with \
         new source. Jupyter notebooks are interactive documents that combine code, text, \
         and visualizations, commonly used for data analysis and scientific computing. The \
         notebook_path parameter must be an absolute path, not a relative path. The \
         cell_number is 0-indexed. Use edit_mode=insert to add a new cell at the index \
         specified by cell_number. Use edit_mode=delete to delete the cell at the index \
         specified by cell_number."
            .to_string()
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
            Some(p) => resolve_notebook_path(p, &_ctx.working_directory),
            None => return Ok(error_result("missing required parameter: notebook_path")),
        };
        let new_source = match input["new_source"].as_str() {
            Some(s) => s.to_string(),
            None => return Ok(error_result("missing required parameter: new_source")),
        };
        let cell_id = input["cell_id"].as_str();
        let mut cell_type = input["cell_type"].as_str().map(str::to_string);
        let original_edit_mode = input["edit_mode"].as_str().unwrap_or("replace");

        // Read notebook JSON
        let raw = match tokio::fs::read_to_string(&notebook_path).await {
            Ok(s) => s,
            Err(e) => {
                return Ok(error_result_with_fields(
                    format!("cannot read notebook '{}': {}", notebook_path, e),
                    &notebook_path,
                    &new_source,
                    cell_type.as_deref().unwrap_or("code"),
                    "python",
                    original_edit_mode,
                    cell_id,
                    "",
                    "",
                ));
            }
        };

        let mut notebook: Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => {
                return Ok(error_result_with_fields(
                    format!("invalid JSON in notebook '{}': {}", notebook_path, e),
                    &notebook_path,
                    &new_source,
                    cell_type.as_deref().unwrap_or("code"),
                    "python",
                    "replace",
                    cell_id,
                    "",
                    "",
                ));
            }
        };

        let language = notebook["metadata"]["language_info"]["name"]
            .as_str()
            .unwrap_or("python")
            .to_string();
        let supports_cell_ids = notebook["nbformat"].as_u64().unwrap_or(4) > 4
            || (notebook["nbformat"].as_u64().unwrap_or(4) == 4
                && notebook["nbformat_minor"].as_u64().unwrap_or(0) >= 5);

        let cells = match notebook["cells"].as_array_mut() {
            Some(c) => c,
            None => {
                return Ok(error_result_with_fields(
                    "notebook does not have a 'cells' array",
                    &notebook_path,
                    &new_source,
                    cell_type.as_deref().unwrap_or("code"),
                    &language,
                    original_edit_mode,
                    cell_id,
                    "",
                    "",
                ));
            }
        };

        let mut cell_index = if let Some(cell_id) = cell_id {
            cells
                .iter()
                .position(|cell| cell["id"].as_str() == Some(cell_id))
                .or_else(|| parse_cell_id(cell_id))
                .unwrap_or(cells.len())
        } else {
            0
        };

        if cell_id.is_none() && original_edit_mode != "insert" {
            return Ok(error_result_with_fields(
                "Cell ID must be specified when not inserting a new cell.",
                &notebook_path,
                &new_source,
                cell_type.as_deref().unwrap_or("code"),
                &language,
                original_edit_mode,
                cell_id,
                "",
                "",
            ));
        }

        if original_edit_mode != "insert" && cell_index >= cells.len() {
            return Ok(error_result_with_fields(
                format!(
                    "Cell with {} not found in notebook.",
                    cell_id
                        .map(|id| format!("ID \"{id}\""))
                        .unwrap_or_else(|| format!("index {cell_index}"))
                ),
                &notebook_path,
                &new_source,
                cell_type.as_deref().unwrap_or("code"),
                &language,
                original_edit_mode,
                cell_id,
                "",
                "",
            ));
        }

        if original_edit_mode == "insert" && cell_id.is_some() {
            cell_index = cell_index.saturating_add(1);
        }

        let mut edit_mode = original_edit_mode.to_string();
        if edit_mode == "replace" && cell_index == cells.len() {
            edit_mode = "insert".to_string();
            if cell_type.is_none() {
                cell_type = Some("code".to_string());
            }
        }

        let new_cell_id = if supports_cell_ids {
            if edit_mode == "insert" {
                Some(
                    uuid::Uuid::new_v4()
                        .simple()
                        .to_string()
                        .chars()
                        .take(13)
                        .collect::<String>(),
                )
            } else {
                cell_id.map(str::to_string)
            }
        } else {
            None
        };

        if edit_mode == "delete" {
            cells.remove(cell_index);
        } else if edit_mode == "insert" {
            let final_cell_type = cell_type.clone().unwrap_or_else(|| "code".to_string());
            let mut new_cell = if final_cell_type == "markdown" {
                json!({
                    "cell_type": "markdown",
                    "source": new_source,
                    "metadata": {},
                })
            } else {
                json!({
                    "cell_type": "code",
                    "source": new_source,
                    "metadata": {},
                    "execution_count": null,
                    "outputs": [],
                })
            };
            if let Some(id) = &new_cell_id {
                new_cell["id"] = json!(id);
            }
            cells.insert(cell_index.min(cells.len()), new_cell);
        } else {
            let cell = &mut cells[cell_index];
            cell["source"] = Value::String(new_source.clone());
            if cell["cell_type"].as_str() == Some("code") {
                cell["execution_count"] = Value::Null;
                cell["outputs"] = json!([]);
            }
            if let Some(ct) = &cell_type {
                cell["cell_type"] = Value::String(ct.clone());
            }
        };

        let updated_json = match serde_json::to_string_pretty(&notebook) {
            Ok(s) => s,
            Err(e) => {
                return Ok(error_result_with_fields(
                    format!("failed to serialize notebook: {}", e),
                    &notebook_path,
                    &new_source,
                    cell_type.as_deref().unwrap_or("code"),
                    &language,
                    &edit_mode,
                    cell_id,
                    &raw,
                    "",
                ));
            }
        };

        if let Err(e) = tokio::fs::write(&notebook_path, &updated_json).await {
            return Ok(error_result_with_fields(
                format!("failed to write notebook '{}': {}", notebook_path, e),
                &notebook_path,
                &new_source,
                cell_type.as_deref().unwrap_or("code"),
                &language,
                &edit_mode,
                cell_id,
                &raw,
                "",
            ));
        }

        if let Ok(mut state) = _ctx.read_file_state.lock() {
            state.update_after_write(&notebook_path, Some(updated_json.clone()));
        }

        Ok(ToolResultData {
            data: json!({
                "new_source": new_source,
                "cell_type": cell_type.unwrap_or_else(|| "code".to_string()),
                "language": language,
                "edit_mode": edit_mode,
                "cell_id": new_cell_id,
                "error": "",
                "notebook_path": notebook_path,
                "original_file": raw,
                "updated_file": updated_json,
            }),
            is_error: false,
        })
    }
}
