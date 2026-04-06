use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use crate::write::FILE_HISTORY;
use claude_core::types::events::ToolResultData;

/// Maximum number of characters shown in error message snippets.
const MAX_DISPLAY_LEN: usize = 100;

/// Truncate a string to at most `MAX_DISPLAY_LEN` characters for display in error messages.
fn truncate_display(s: &str) -> String {
    if s.len() <= MAX_DISPLAY_LEN {
        s.to_string()
    } else {
        format!("{}…", &s[..MAX_DISPLAY_LEN])
    }
}

fn error_result(msg: impl Into<String>) -> ToolResultData {
    ToolResultData {
        data: json!({ "error": msg.into() }),
        is_error: true,
    }
}

pub struct FileEditTool;

#[async_trait]
impl ToolExecutor for FileEditTool {
    fn name(&self) -> &str {
        "Edit"
    }

    fn description(&self) -> String {
        r#"Performs exact string replacements in files.

Usage:
- You must use your `Read` tool at least once in the conversation before editing. This tool will error if you attempt an edit without reading the file.
- When editing text from Read tool output, ensure you preserve the exact indentation (tabs/spaces) as it appears AFTER the line number prefix. The line number prefix format is: line number + tab. Everything after that is the actual file content to match. Never include any part of the line number prefix in the old_string or new_string.
- ALWAYS prefer editing existing files in the codebase. NEVER write new files unless explicitly required.
- Only use emojis if the user explicitly requests it. Avoid adding emojis to files unless asked.
- The edit will FAIL if `old_string` is not unique in the file. Either provide a larger string with more surrounding context to make it unique or use `replace_all` to change every instance of `old_string`.
- Use `replace_all` for replacing and renaming strings across the file. This parameter is useful if you want to rename a variable for instance."#.to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the file to edit"
                },
                "old_string": {
                    "type": "string",
                    "description": "The string to search for and replace"
                },
                "new_string": {
                    "type": "string",
                    "description": "The string to replace old_string with"
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "If true, replace all occurrences; otherwise require exactly one match",
                    "default": false
                }
            },
            "required": ["file_path", "old_string", "new_string"]
        })
    }

    async fn call(
        &self,
        input: &Value,
        ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let file_path = match input["file_path"].as_str() {
            Some(p) => p,
            None => return Ok(error_result("Missing required field: file_path")),
        };
        let old_string = match input["old_string"].as_str() {
            Some(s) => s,
            None => return Ok(error_result("Missing required field: old_string")),
        };
        let new_string = match input["new_string"].as_str() {
            Some(s) => s,
            None => return Ok(error_result("Missing required field: new_string")),
        };
        let replace_all = input["replace_all"].as_bool().unwrap_or(false);

        let path = std::path::Path::new(file_path);

        // Guard: refuse to raw-edit Jupyter notebook files. Raw string replacement
        // inside notebook JSON can corrupt cell metadata, output arrays, and the
        // nbformat schema. Mirrors TS FileEditTool lines 266-273.
        if path.extension().map_or(false, |ext| ext == "ipynb") {
            return Ok(error_result(
                "File is a Jupyter Notebook (.ipynb). Use the NotebookEdit tool to edit \
                 notebook cells instead. Raw string replacement in notebook JSON can \
                 corrupt cell metadata and break the notebook format.",
            ));
        }

        // If old_string is non-empty and the file doesn't exist -> error
        if !path.exists() {
            if old_string.is_empty() {
                // Creating a new file with no old content to replace -- write empty->new
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(path, new_string)?;
                return Ok(ToolResultData {
                    data: json!({
                        "filePath": file_path,
                        "oldString": old_string,
                        "newString": new_string,
                        "originalFile": "",
                        "replaceAll": replace_all
                    }),
                    is_error: false,
                });
            }
            return Ok(error_result(format!("File not found: {}", file_path)));
        }

        // Staleness check: ensure the file has been read and not modified since.
        if let Err(msg) = crate::write::check_file_staleness(file_path, path, &ctx.read_file_state)
        {
            return Ok(error_result(msg));
        }

        // Take a snapshot before editing.
        if let Ok(mut tracker) = FILE_HISTORY.lock() {
            let _ = tracker.snapshot(path);
        }

        let original = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => return Ok(error_result(format!("Failed to read file: {}", e))),
        };

        // Count occurrences
        let count = original.matches(old_string).count();

        if count == 0 {
            return Ok(error_result(format!(
                "String not found in file.\nSearched for: {}",
                truncate_display(old_string)
            )));
        }

        if count > 1 && !replace_all {
            return Ok(error_result(format!(
                "Found {} occurrences of the search string but replace_all is false. \
                 Use replace_all=true to replace all occurrences, or provide a more specific \
                 old_string that matches exactly once.\nSearched for: {}",
                count,
                truncate_display(old_string)
            )));
        }

        let new_content = if replace_all {
            original.replace(old_string, new_string)
        } else {
            // replace first occurrence only
            original.replacen(old_string, new_string, 1)
        };

        // Ensure parent directories exist (in case of a new path -- defensive)
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent)?;
            }
        }

        if let Err(e) = std::fs::write(path, &new_content) {
            return Ok(error_result(format!("Failed to write file: {}", e)));
        }

        // Update read state after successful edit
        if let Ok(mut state) = ctx.read_file_state.lock() {
            state.update_after_write(file_path);
        }

        Ok(ToolResultData {
            data: json!({
                "filePath": file_path,
                "oldString": old_string,
                "newString": new_string,
                "originalFile": original,
                "replaceAll": replace_all
            }),
            is_error: false,
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

    fn max_result_size_chars(&self) -> usize {
        100_000
    }
}
