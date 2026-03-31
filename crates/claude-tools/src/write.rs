use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ReadFileState, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

/// Check whether a file has been read and has not been externally modified
/// since that read. Returns `Err(message)` if the write should be rejected.
///
/// Mirrors the TS `validateInput` in `FileWriteTool.ts`:
/// - Error if file has never been read (readFileState has no entry)
/// - Error if the read was a partial view (offset/limit supplied)
/// - Error if the file's mtime is newer than the recorded read timestamp
pub fn check_file_staleness(
    file_path: &str,
    path: &std::path::Path,
    read_state: &Arc<Mutex<ReadFileState>>,
) -> std::result::Result<(), String> {
    let state = read_state.lock().map_err(|_| "internal error: lock poisoned".to_string())?;

    let entry = match state.get(file_path) {
        Some(e) => e,
        None => {
            return Err(
                "File has not been read yet. Read it first before writing to it.".to_string(),
            );
        }
    };

    if entry.is_partial_view {
        return Err(
            "File has not been read yet. Read it first before writing to it.".to_string(),
        );
    }

    // Check mtime
    if let Ok(metadata) = path.metadata() {
        if let Ok(modified) = metadata.modified() {
            let mtime_ms = modified
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            if mtime_ms > entry.timestamp {
                return Err(
                    "File has been modified since read, either by the user or by a linter. \
                     Read it again before attempting to write it."
                        .to_string(),
                );
            }
        }
    }

    Ok(())
}

pub struct FileWriteTool;

#[async_trait]
impl ToolExecutor for FileWriteTool {
    fn name(&self) -> &str {
        "Write"
    }

    fn description(&self) -> String {
        r#"Writes a file to the local filesystem.

Usage:
- This tool will overwrite the existing file if there is one at the provided path.
- If this is an existing file, you MUST use the Read tool first to read the file's contents. This tool will fail if you did not read the file first.
- Prefer the Edit tool for modifying existing files -- it only sends the diff. Only use this tool to create new files or for complete rewrites.
- NEVER create documentation files (*.md) or README files unless explicitly requested by the User.
- Only use emojis if the user explicitly requests it. Avoid writing emojis to files unless asked."#.to_string()
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
        ctx: &ToolUseContext,
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

        // Staleness check: if the file already exists, ensure it has been read
        // first and has not been modified since the last read.
        if path.exists() {
            if let Err(msg) = check_file_staleness(file_path, path, &ctx.read_file_state) {
                return Ok(ToolResultData {
                    data: json!({ "error": msg }),
                    is_error: true,
                });
            }
        }

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

        // Update read state after successful write so subsequent writes are not rejected
        if let Ok(mut state) = ctx.read_file_state.lock() {
            state.update_after_write(file_path);
        }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_write_without_read_rejected() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path();
        std::fs::write(path, "original").unwrap();

        let state = Arc::new(Mutex::new(ReadFileState::new()));
        let result = check_file_staleness(
            path.to_str().unwrap(),
            path,
            &state,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not been read yet"));
    }

    #[test]
    fn test_write_after_read_accepted() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path();
        std::fs::write(path, "original").unwrap();

        let state = Arc::new(Mutex::new(ReadFileState::new()));
        // Simulate a read
        state.lock().unwrap().record_read(path.to_str().unwrap(), false);

        let result = check_file_staleness(
            path.to_str().unwrap(),
            path,
            &state,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_write_after_modification_rejected() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path();
        std::fs::write(path, "original").unwrap();

        let state = Arc::new(Mutex::new(ReadFileState::new()));
        // Record read with an old timestamp
        {
            let mut s = state.lock().unwrap();
            s.insert_entry(
                path.to_str().unwrap(),
                crate::registry::ReadFileEntry {
                    timestamp: 1000, // very old timestamp
                    is_partial_view: false,
                },
            );
        }

        // File's mtime will be newer than timestamp 1000
        let result = check_file_staleness(
            path.to_str().unwrap(),
            path,
            &state,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("modified since read"));
    }

    #[test]
    fn test_partial_read_rejected() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path();
        std::fs::write(path, "original").unwrap();

        let state = Arc::new(Mutex::new(ReadFileState::new()));
        // Record a partial read
        state.lock().unwrap().record_read(path.to_str().unwrap(), true);

        let result = check_file_staleness(
            path.to_str().unwrap(),
            path,
            &state,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not been read yet"));
    }
}
