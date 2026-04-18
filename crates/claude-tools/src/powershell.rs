use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

const MAX_OUTPUT_CHARS: usize = 30_000;

/// Verbatim port of TS PowerShellTool/prompt.ts getPrompt(). TS
/// branches on `getPowerShellEdition()`; the Rust port embeds the
/// conservative "unknown — assume 5.1" branch because
/// edition-detection requires a live pwsh call that hasn't landed
/// yet. Callers that want the 7+ guidance should inject their own
/// edition-aware prompt. TS `${getMaxOutputLength()}` +
/// `${getMaxTimeoutMs()}` are baked in at 600_000ms max / 120_000ms
/// default (the static defaults in utils/timeouts.ts).
pub const POWERSHELL_PROMPT: &str = include_str!("prompts/powershell.md");

pub struct PowerShellTool;

fn truncate_output(s: String) -> String {
    if s.len() <= MAX_OUTPUT_CHARS {
        s
    } else {
        s[..MAX_OUTPUT_CHARS].to_string()
    }
}

#[async_trait]
impl ToolExecutor for PowerShellTool {
    fn name(&self) -> &str {
        "PowerShell"
    }

    fn description(&self) -> String {
        POWERSHELL_PROMPT.to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The PowerShell command to execute."
                },
                "timeout": {
                    "type": "integer",
                    "description": "Optional timeout in milliseconds (max 600000)."
                }
            },
            "required": ["command"]
        })
    }

    async fn call(
        &self,
        input: &Value,
        ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let command = match input.get("command").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => {
                return Ok(ToolResultData {
                    data: json!({ "error": "missing required field: command" }),
                    is_error: true,
                });
            }
        };

        if command.trim().is_empty() {
            return Ok(ToolResultData {
                data: json!({ "error": "command must not be empty" }),
                is_error: true,
            });
        }

        // PowerShell is only available on Windows
        if cfg!(not(target_os = "windows")) {
            return Ok(ToolResultData {
                data: json!({
                    "error": "PowerShell tool is only available on Windows. Use the Bash tool instead."
                }),
                is_error: true,
            });
        }

        let _timeout_ms = input
            .get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(120_000)
            .min(600_000);

        let output = tokio::process::Command::new("powershell")
            .args(["-Command", command])
            .current_dir(&ctx.working_directory)
            .output()
            .await;

        match output {
            Ok(output) => {
                let stdout = truncate_output(String::from_utf8_lossy(&output.stdout).to_string());
                let stderr = truncate_output(String::from_utf8_lossy(&output.stderr).to_string());
                let exit_code = output.status.code().unwrap_or(-1);

                Ok(ToolResultData {
                    data: json!({
                        "stdout": stdout,
                        "stderr": stderr,
                        "exitCode": exit_code
                    }),
                    is_error: exit_code != 0,
                })
            }
            Err(e) => Ok(ToolResultData {
                data: json!({ "error": format!("Failed to execute PowerShell: {}", e) }),
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
            permission_mode: crate::registry::PermissionMode::Default,
        }
    }

    #[tokio::test]
    async fn powershell_not_available_on_non_windows() {
        if cfg!(target_os = "windows") {
            return; // skip on actual Windows
        }
        let tool = PowerShellTool;
        let input = json!({ "command": "Get-Process" });
        let ctx = make_ctx();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(result.is_error);
        assert!(result.data["error"]
            .as_str()
            .unwrap()
            .contains("only available on Windows"));
    }

    #[tokio::test]
    async fn powershell_missing_command() {
        let tool = PowerShellTool;
        let input = json!({});
        let ctx = make_ctx();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(result.is_error);
        assert!(result.data["error"]
            .as_str()
            .unwrap()
            .contains("missing required field: command"));
    }

    #[tokio::test]
    async fn powershell_empty_command() {
        let tool = PowerShellTool;
        let input = json!({ "command": "   " });
        let ctx = make_ctx();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(result.is_error);
        assert!(result.data["error"]
            .as_str()
            .unwrap()
            .contains("must not be empty"));
    }
}
