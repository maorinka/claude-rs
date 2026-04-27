use anyhow::{Context, Result};
use async_trait::async_trait;
use once_cell::sync::Lazy;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ReadFileState, ToolExecutor, ToolUseContext};
use claude_core::file_history::FileHistoryTracker;
use claude_core::types::events::ToolResultData;

// ─── Global file-history tracker ──────────────────────────────────────────────
//
// Uses a temp-directory-based session dir by default.  Callers that want a
// specific session dir should initialise `FILE_HISTORY_SESSION_DIR` before the
// first write.

pub(crate) static FILE_HISTORY: Lazy<Mutex<FileHistoryTracker>> = Lazy::new(|| {
    let session_dir = std::env::temp_dir().join("claude_rs_session");
    Mutex::new(FileHistoryTracker::new(&session_dir))
});

/// Override the session directory used for file snapshots.
/// Must be called before the first snapshot is taken to have any effect.
pub fn set_file_history_session_dir(session_dir: &std::path::Path) {
    if let Ok(mut tracker) = FILE_HISTORY.lock() {
        *tracker = FileHistoryTracker::new(session_dir);
    }
}

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
    let state = read_state
        .lock()
        .map_err(|_| "internal error: lock poisoned".to_string())?;

    let entry = match state.get(file_path) {
        Some(e) => e,
        None => {
            return Err(
                "File has not been read yet. Read it first before writing to it.".to_string(),
            );
        }
    };

    if entry.is_partial_view {
        return Err("File has not been read yet. Read it first before writing to it.".to_string());
    }

    // Check mtime
    if let Ok(metadata) = path.metadata() {
        if let Ok(modified) = metadata.modified() {
            let mtime_ms = modified
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            if mtime_ms > entry.timestamp {
                // mtime changed — for full reads, compare content as a fallback.
                // Antivirus/cloud-sync can touch mtime without actually changing content.
                // Mirrors TS FileWriteTool.ts:282-294.
                if let Some(ref stored_content) = entry.content {
                    if let Ok(current_content) = std::fs::read_to_string(path) {
                        // Normalize CRLF -> LF for comparison (matches TS normalisation).
                        let normalized_current = current_content.replace("\r\n", "\n");
                        let normalized_stored = stored_content.replace("\r\n", "\n");
                        if normalized_current == normalized_stored {
                            // Content unchanged — mtime touch was harmless, allow write.
                            return Ok(());
                        }
                    }
                }
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

        // Team-memory secret guard — reject writes that would leak secrets
        // into a team-memory file synced to all collaborators. Runs BEFORE
        // staleness to match TS `FileWriteTool.validateInput` (which calls
        // checkTeamMemSecrets first, then staleness via the call() path).
        // cwd comes from the tool context (request-scoped, equivalent to
        // TS's AsyncLocalStorage-backed getCwd()) — not ambient process
        // state. The guard short-circuits cheaply when TEAMMEM is off or
        // the path isn't a team-memory path.
        if let Some(msg) = claude_core::teams::team_mem_secret_guard::check_team_mem_secrets(
            path,
            content,
            &ctx.working_directory,
        ) {
            return Ok(ToolResultData {
                data: json!({ "error": msg }),
                is_error: true,
            });
        }

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

        // Take a snapshot of the file before overwriting.
        if let Ok(mut tracker) = FILE_HISTORY.lock() {
            let _ = tracker.snapshot(path);
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

        let write_type = if original_file.is_some() {
            "update"
        } else {
            "create"
        };

        // Create parent directories if they do not exist.
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create parent dirs for: {file_path}"))?;
            }
        }

        std::fs::write(path, content)
            .with_context(|| format!("failed to write file: {file_path}"))?;

        // Update read state after successful write so subsequent writes are not rejected.
        // Store the raw `content` that was written — matches TS `FileWriteTool.ts:331`
        // which stores the argument passed to `writeTextContent` without
        // LF-normalisation. `check_file_staleness` at write.rs:73-78 normalises both
        // sides before comparing, so storage-form doesn't affect correctness; this
        // keeps exact TS storage parity.
        if let Ok(mut state) = ctx.read_file_state.lock() {
            state.update_after_write(file_path, Some(content.to_string()));
        }

        let data = json!({
            "type": write_type,
            "filePath": file_path,
            "content": content,
            "originalFile": original_file,
        });

        Ok(ToolResultData {
            data,
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

    fn check_permissions(
        &self,
        input: &Value,
        context: &claude_core::permissions::ToolPermissionContext,
    ) -> claude_core::permissions::PermissionResult {
        let Some(file_path) = input["file_path"].as_str() else {
            return claude_core::permissions::PermissionResult::passthrough("");
        };
        match claude_core::permissions::check_write_permission_for_tool(file_path, context) {
            claude_core::permissions::PermissionDecision::Allow(allow) => {
                claude_core::permissions::PermissionResult::Allow(allow)
            }
            claude_core::permissions::PermissionDecision::Ask(ask) => {
                claude_core::permissions::PermissionResult::Ask(ask)
            }
            claude_core::permissions::PermissionDecision::Deny(deny) => {
                claude_core::permissions::PermissionResult::Deny(deny)
            }
        }
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
        let result = check_file_staleness(path.to_str().unwrap(), path, &state);
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
        state
            .lock()
            .unwrap()
            .record_read(path.to_str().unwrap(), false, None);

        let result = check_file_staleness(path.to_str().unwrap(), path, &state);
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
                    content: None,
                },
            );
        }

        // File's mtime will be newer than timestamp 1000
        let result = check_file_staleness(path.to_str().unwrap(), path, &state);
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
        state
            .lock()
            .unwrap()
            .record_read(path.to_str().unwrap(), true, None);

        let result = check_file_staleness(path.to_str().unwrap(), path, &state);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not been read yet"));
    }
}
