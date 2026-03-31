use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use claude_core::types::events::ToolResultData;
use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};

/// Paths that must never be opened (infinite / blocking / sensitive device files).
const BLOCKED_PATHS: &[&str] = &[
    "/dev/zero",
    "/dev/random",
    "/dev/urandom",
    "/dev/stdin",
    "/dev/tty",
    "/dev/null",
];

const DEFAULT_LINE_LIMIT: u64 = 2000;

pub struct FileReadTool;

#[async_trait]
impl ToolExecutor for FileReadTool {
    fn name(&self) -> &str {
        "Read"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the file to read."
                },
                "offset": {
                    "type": "integer",
                    "description": "0-based line index to start reading from."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of lines to return."
                },
                "pages": {
                    "type": "string",
                    "description": "Page range for PDF files (e.g. \"1-5\")."
                }
            },
            "required": ["file_path"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    fn is_read_only(&self, _input: &Value) -> bool {
        true
    }

    fn max_result_size_chars(&self) -> usize {
        usize::MAX
    }

    async fn call(
        &self,
        input: &Value,
        _ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let file_path = match input["file_path"].as_str() {
            Some(p) => p,
            None => {
                return Ok(error_result("missing required parameter: file_path"));
            }
        };

        // Block dangerous device paths.
        if BLOCKED_PATHS.contains(&file_path) {
            return Ok(error_result(&format!(
                "access to '{}' is blocked for safety reasons",
                file_path
            )));
        }

        let offset = input["offset"].as_u64().unwrap_or(0) as usize;
        let limit = input["limit"].as_u64().unwrap_or(DEFAULT_LINE_LIMIT) as usize;

        // Read the file, returning an error result if it does not exist / cannot be read.
        let raw = match tokio::fs::read_to_string(file_path).await {
            Ok(s) => s,
            Err(e) => {
                return Ok(error_result(&format!(
                    "cannot read '{}': {}",
                    file_path, e
                )));
            }
        };

        // Split into lines preserving content (strip the trailing newline if present so we
        // don't get a spurious empty line at the end).
        let all_lines: Vec<&str> = raw.lines().collect();
        let total_lines = all_lines.len();

        // Apply offset and limit.
        let start = offset.min(total_lines);
        let end = (start + limit).min(total_lines);
        let selected = &all_lines[start..end];

        // Format in cat -n style: "{1-based-line-num}\t{content}"
        let start_line = start + 1; // convert to 1-based
        let mut formatted = String::new();
        for (i, line) in selected.iter().enumerate() {
            let line_num = start_line + i;
            formatted.push_str(&format!("{}\t{}\n", line_num, line));
        }

        let result_data = json!({
            "type": "text",
            "file": {
                "filePath": file_path,
                "content": formatted,
                "numLines": selected.len(),
                "startLine": start_line,
                "totalLines": total_lines
            }
        });

        Ok(ToolResultData {
            data: result_data,
            is_error: false,
        })
    }
}

fn error_result(msg: &str) -> ToolResultData {
    ToolResultData {
        data: json!({ "error": msg }),
        is_error: true,
    }
}
