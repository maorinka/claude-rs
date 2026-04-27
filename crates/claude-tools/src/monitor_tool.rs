use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use crate::task_tools::{append_output, create_task_entry, register_process, update_task_status};
use claude_core::types::events::ToolResultData;

const DEFAULT_TIMEOUT_MS: u64 = 300_000;
const MAX_TIMEOUT_MS: u64 = 3_600_000;

pub struct MonitorTool;

#[async_trait]
impl ToolExecutor for MonitorTool {
    fn name(&self) -> &str {
        "Monitor"
    }

    fn description(&self) -> String {
        "Start a background monitor that streams events from a long-running script. Each stdout line is an event; exit ends the watch.".to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "description": {
                    "type": "string",
                    "description": "Short human-readable description of what you are monitoring (shown in notifications)."
                },
                "timeout_ms": {
                    "type": "number",
                    "minimum": 1000,
                    "default": DEFAULT_TIMEOUT_MS,
                    "description": "Kill the monitor after this deadline. Default 300000ms, max 3600000ms. Ignored when persistent is true."
                },
                "persistent": {
                    "type": "boolean",
                    "default": false,
                    "description": "Run for the lifetime of the session (no timeout). Use for session-length watches like PR monitoring or log tails. Stop with TaskStop."
                },
                "command": {
                    "type": "string",
                    "description": "Shell command or script. Each stdout line is an event; exit ends the watch."
                }
            },
            "required": ["description", "timeout_ms", "persistent", "command"]
        })
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
        let Some(command) = input.get("command").and_then(|value| value.as_str()) else {
            return Ok(error_result("missing required field: command"));
        };
        if command.trim().is_empty() {
            return Ok(error_result("command must not be empty"));
        }

        let Some(description) = input.get("description").and_then(|value| value.as_str()) else {
            return Ok(error_result("missing required field: description"));
        };
        if description.trim().is_empty() {
            return Ok(error_result("description must not be empty"));
        }

        let persistent = input
            .get("persistent")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let timeout_ms = input
            .get("timeout_ms")
            .and_then(|value| value.as_u64())
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .clamp(1_000, MAX_TIMEOUT_MS);

        let mut child = tokio::process::Command::new("bash")
            .arg("-c")
            .arg(command)
            .current_dir(&ctx.working_directory)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let pid = child.id().unwrap_or(0);
        let task_id = create_task_entry("Monitor", description);
        register_process(&task_id, pid);

        let output_dir = std::env::temp_dir().join("claude-bg-tasks");
        let _ = std::fs::create_dir_all(&output_dir);
        let output_path = output_dir.join(format!("{task_id}.output"));
        let output_path_string = output_path.to_string_lossy().to_string();

        let stdout = child.stdout.take().expect("stdout pipe");
        let stderr = child.stderr.take().expect("stderr pipe");
        let task_id_for_task = task_id.clone();
        let output_path_for_task = output_path.clone();

        tokio::spawn(async move {
            let file = tokio::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&output_path_for_task)
                .await;

            let Ok(file) = file else {
                wait_for_monitor(child, task_id_for_task, persistent, timeout_ms).await;
                return;
            };

            let file = std::sync::Arc::new(tokio::sync::Mutex::new(file));
            let stdout_task_id = task_id_for_task.clone();
            let stdout_file = file.clone();
            let stdout_task = tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let event = format!("{line}\n");
                    append_output(&stdout_task_id, &event);
                    let mut file = stdout_file.lock().await;
                    let _ = file.write_all(event.as_bytes()).await;
                    let _ = file.flush().await;
                }
            });

            let stderr_file = file.clone();
            let stderr_task = tokio::spawn(async move {
                let mut reader = stderr;
                let mut buf = vec![0u8; 4096];
                loop {
                    match reader.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let mut file = stderr_file.lock().await;
                            let _ = file.write_all(&buf[..n]).await;
                            let _ = file.flush().await;
                        }
                    }
                }
            });

            wait_for_monitor(child, task_id_for_task, persistent, timeout_ms).await;
            let _ = stdout_task.await;
            let _ = stderr_task.await;
        });

        Ok(ToolResultData {
            data: json!({
                "stdout": "",
                "stderr": "",
                "code": 0,
                "interrupted": false,
                "task_id": task_id,
                "taskId": task_id,
                "backgroundTaskId": task_id,
                "description": description,
                "status": "running",
                "persistent": persistent,
                "timeout_ms": timeout_ms,
                "output_file": output_path_string,
                "outputPath": output_path_string,
                "pid": pid
            }),
            is_error: false,
        })
    }
}

async fn wait_for_monitor(
    mut child: tokio::process::Child,
    task_id: String,
    persistent: bool,
    timeout_ms: u64,
) {
    let status = if persistent {
        child.wait().await.ok()
    } else {
        match tokio::time::timeout(std::time::Duration::from_millis(timeout_ms), child.wait()).await
        {
            Ok(status) => status.ok(),
            Err(_) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                update_task_status(&task_id, "stopped");
                return;
            }
        }
    };

    match status.and_then(|status| status.code()) {
        Some(0) => update_task_status(&task_id, "completed"),
        _ => update_task_status(&task_id, "failed"),
    }
}

fn error_result(message: impl Into<String>) -> ToolResultData {
    ToolResultData {
        data: json!({ "error": message.into() }),
        is_error: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ReadFileState;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn make_ctx() -> ToolUseContext {
        ToolUseContext::for_test(
            PathBuf::from("/tmp"),
            Arc::new(std::sync::Mutex::new(ReadFileState::new())),
            crate::registry::PermissionMode::Default,
        )
    }

    #[tokio::test]
    async fn monitor_launches_background_command() {
        let result = MonitorTool
            .call(
                &json!({
                    "description": "test monitor",
                    "timeout_ms": 1000,
                    "persistent": false,
                    "command": "printf 'ready\\n'"
                }),
                &make_ctx(),
                CancellationToken::new(),
                None,
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        assert_eq!(result.data["status"], "running");
        assert!(result.data["task_id"].as_str().is_some());
        assert!(result.data["output_file"].as_str().is_some());
    }

    #[tokio::test]
    async fn monitor_requires_command_and_description() {
        let result = MonitorTool
            .call(
                &json!({"description": "missing command", "timeout_ms": 1000, "persistent": false}),
                &make_ctx(),
                CancellationToken::new(),
                None,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.data["error"].as_str().unwrap().contains("command"));
    }

    #[tokio::test]
    async fn monitor_tool_properties() {
        assert_eq!(MonitorTool.name(), "Monitor");
        assert!(!MonitorTool.is_read_only(&json!({})));
        assert!(MonitorTool.is_concurrency_safe(&json!({})));
    }
}
