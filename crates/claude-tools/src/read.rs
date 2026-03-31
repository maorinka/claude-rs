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

    fn description(&self) -> String {
        r#"Reads a file from the local filesystem. You can access any file directly by using this tool.
Assume this tool is able to read all files on the machine. If the User provides a path to a file assume that path is valid. It is okay to read a file that does not exist; an error will be returned.

Usage:
- The file_path parameter must be an absolute path, not a relative path
- By default, it reads up to 2000 lines starting from the beginning of the file
- When you already know which part of the file you need, only read that part. This can be important for larger files.
- Results are returned using cat -n format, with line numbers starting at 1
- This tool allows Claude Code to read images (eg PNG, JPG, etc). When reading an image file the contents are presented visually as Claude Code is a multimodal LLM.
- This tool can read PDF files (.pdf). For large PDFs (more than 10 pages), you MUST provide the pages parameter to read specific page ranges (e.g., pages: "1-5"). Reading a large PDF without the pages parameter will fail. Maximum 20 pages per request.
- This tool can read Jupyter notebooks (.ipynb files) and returns all cells with their outputs, combining code, text, and visualizations.
- This tool can only read files, not directories. To read a directory, use an ls command via the Bash tool.
- You will regularly be asked to read screenshots. If the user provides a path to a screenshot, ALWAYS use this tool to view the file at the path. This tool will work with all temporary file paths.
- If you read a file that exists but has empty contents you will receive a system reminder warning in place of file contents."#.to_string()
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
        ctx: &ToolUseContext,
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

        // Record this read in the shared state for staleness tracking.
        let is_partial = offset > 0 || (limit as u64) < DEFAULT_LINE_LIMIT;
        if let Ok(mut state) = ctx.read_file_state.lock() {
            state.record_read(file_path, is_partial);
        }

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
