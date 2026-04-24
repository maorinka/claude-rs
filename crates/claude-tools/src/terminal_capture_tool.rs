use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

pub struct TerminalCaptureTool;

#[async_trait]
impl ToolExecutor for TerminalCaptureTool {
    fn name(&self) -> &str {
        "TerminalCapture"
    }
    fn description(&self) -> String {
        "Capture the current terminal screen content. Returns the visible text in the terminal. Works best inside tmux or screen.".to_string()
    }
    fn input_schema(&self) -> Value {
        json!({ "type": "object", "properties": { "lines": { "type": "integer", "description": "Number of lines to capture." }, "pane": { "type": "string", "description": "Tmux pane target." } }, "required": [] })
    }
    fn is_read_only(&self, _input: &Value) -> bool {
        true
    }
    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    async fn call(
        &self,
        input: &Value,
        _ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let lines = input
            .get("lines")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize);
        let pane = input.get("pane").and_then(|v| v.as_str());
        if std::env::var("TMUX").is_ok() {
            let mut args = vec![
                "capture-pane".to_string(),
                "-p".to_string(),
                "-J".to_string(),
            ];
            if let Some(t) = pane {
                args.push("-t".to_string());
                args.push(t.to_string());
            }
            if let Some(n) = lines {
                args.push("-S".to_string());
                args.push(format!("-{}", n));
            }
            match tokio::process::Command::new("tmux")
                .args(&args)
                .output()
                .await
            {
                Ok(out) if out.status.success() => {
                    let content = String::from_utf8_lossy(&out.stdout).to_string();
                    return Ok(ToolResultData {
                        data: json!({ "content": content, "lines": content.lines().count(), "method": "tmux", "pane": pane }),
                        is_error: false,
                    });
                }
                Ok(out) => {
                    return Ok(ToolResultData {
                        data: json!({ "error": format!("tmux capture failed: {}", String::from_utf8_lossy(&out.stderr)), "method": "tmux" }),
                        is_error: true,
                    })
                }
                Err(e) => {
                    return Ok(ToolResultData {
                        data: json!({ "error": format!("Failed to run tmux: {}", e), "method": "tmux" }),
                        is_error: true,
                    })
                }
            }
        }
        let rows: u16 = std::env::var("LINES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(24);
        let cols: u16 = std::env::var("COLUMNS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(80);
        Ok(ToolResultData {
            data: json!({ "content": null, "terminalSize": { "rows": rows, "cols": cols }, "method": "unavailable", "message": "Terminal capture requires tmux or screen." }),
            is_error: false,
        })
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
    async fn terminal_capture_without_tmux() {
        std::env::remove_var("TMUX");
        let r = TerminalCaptureTool
            .call(&json!({}), &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();
        assert!(!r.is_error);
        assert_eq!(r.data["method"].as_str().unwrap(), "unavailable");
    }
    #[tokio::test]
    async fn terminal_capture_properties() {
        assert_eq!(TerminalCaptureTool.name(), "TerminalCapture");
        assert!(TerminalCaptureTool.is_read_only(&json!({})));
    }
}
