use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

pub struct SendUserFileTool;

#[async_trait]
impl ToolExecutor for SendUserFileTool {
    fn name(&self) -> &str {
        "SendUserFile"
    }
    fn description(&self) -> String {
        "Send a file to the user by copying it to the output directory. The file is copied -- not moved -- so the original remains in place.".to_string()
    }
    fn input_schema(&self) -> Value {
        json!({ "type": "object", "properties": { "filePath": { "type": "string", "description": "Absolute path to the file to send." }, "description": { "type": "string", "description": "Brief description of what the file contains." } }, "required": ["filePath"] })
    }
    fn is_read_only(&self, _input: &Value) -> bool {
        false
    }
    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    async fn call(
        &self,
        input: &Value,
        ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let file_path = match input.get("filePath").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => {
                return Ok(ToolResultData {
                    data: json!({ "error": "missing required field: filePath" }),
                    is_error: true,
                })
            }
        };
        let desc = input
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let source = if std::path::Path::new(file_path).is_absolute() {
            std::path::PathBuf::from(file_path)
        } else {
            ctx.working_directory.join(file_path)
        };
        if !source.exists() {
            return Ok(ToolResultData {
                data: json!({ "error": format!("File not found: {}", source.display()) }),
                is_error: true,
            });
        }
        let output_dir = std::env::var("CLAUDE_OUTPUT_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| {
                let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
                std::path::PathBuf::from(home)
                    .join(".claude")
                    .join("output")
            });
        if let Err(e) = tokio::fs::create_dir_all(&output_dir).await {
            return Ok(ToolResultData {
                data: json!({ "error": format!("Failed to create output directory: {}", e) }),
                is_error: true,
            });
        }
        let file_name = source
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".to_string());
        let mut dest = output_dir.join(&file_name);
        if dest.exists() {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            let stem = source
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "file".to_string());
            let ext = source
                .extension()
                .map(|e| format!(".{}", e.to_string_lossy()))
                .unwrap_or_default();
            dest = output_dir.join(format!("{}_{}{}", stem, ts, ext));
        }
        match tokio::fs::copy(&source, &dest).await {
            Ok(bytes) => Ok(ToolResultData {
                data: json!({ "sent": true, "source": source.display().to_string(), "destination": dest.display().to_string(), "bytes": bytes, "description": desc, "message": format!("File sent to {}", dest.display()) }),
                is_error: false,
            }),
            Err(e) => Ok(ToolResultData {
                data: json!({ "error": format!("Failed to copy file: {}", e) }),
                is_error: true,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ReadFileState;
    use std::path::PathBuf;
    use std::sync::Arc;
    fn make_ctx() -> ToolUseContext {
        ToolUseContext {
            working_directory: PathBuf::from("/tmp"),
            read_file_state: Arc::new(std::sync::Mutex::new(ReadFileState::new())),
        }
    }
    #[tokio::test]
    async fn send_user_file_missing_path() {
        let r = SendUserFileTool
            .call(&json!({}), &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();
        assert!(r.is_error);
        assert!(r.data["error"].as_str().unwrap().contains("filePath"));
    }
    #[tokio::test]
    async fn send_user_file_nonexistent() {
        let r = SendUserFileTool
            .call(
                &json!({ "filePath": "/tmp/definitely_does_not_exist_12345.txt" }),
                &make_ctx(),
                CancellationToken::new(),
                None,
            )
            .await
            .unwrap();
        assert!(r.is_error);
        assert!(r.data["error"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("not found"));
    }
    #[tokio::test]
    async fn send_user_file_properties() {
        assert_eq!(SendUserFileTool.name(), "SendUserFile");
        assert!(!SendUserFileTool.is_read_only(&json!({})));
    }
}
