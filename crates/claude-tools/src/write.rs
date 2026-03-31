use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

pub struct FileWriteTool;

#[async_trait]
impl ToolExecutor for FileWriteTool {
    fn name(&self) -> &str {
        "Write"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path of the file to write."
                },
                "content": {
                    "type": "string",
                    "description": "Content to write to the file."
                }
            },
            "required": ["file_path", "content"]
        })
    }

    async fn call(
        &self,
        input: &Value,
        _ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let file_path = input["file_path"]
            .as_str()
            .context("file_path must be a string")?;
        let content = input["content"]
            .as_str()
            .context("content must be a string")?;

        let path = std::path::Path::new(file_path);

        // Read existing content before overwriting, if any.
        let original_file: Option<String> = if path.exists() {
            Some(
                std::fs::read_to_string(path)
                    .with_context(|| format!("failed to read existing file: {file_path}"))?,
            )
        } else {
            None
        };

        let write_type = if original_file.is_some() { "update" } else { "create" };

        // Create parent directories if they do not exist.
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create parent dirs for: {file_path}"))?;
            }
        }

        std::fs::write(path, content)
            .with_context(|| format!("failed to write file: {file_path}"))?;

        let data = json!({
            "type": write_type,
            "filePath": file_path,
            "content": content,
            "originalFile": original_file,
        });

        Ok(ToolResultData { data, is_error: false })
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
