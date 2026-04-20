use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

pub struct ListPeersTool;

#[async_trait]
impl ToolExecutor for ListPeersTool {
    fn name(&self) -> &str {
        "ListPeers"
    }
    fn description(&self) -> String {
        "List peer Claude Code sessions running on this machine. Returns each peer's PID, session ID, working directory, start time, and optional name. Stale sessions (dead processes) are automatically cleaned up.".to_string()
    }
    fn input_schema(&self) -> Value {
        json!({ "type": "object", "properties": {}, "required": [] })
    }
    fn is_read_only(&self, _input: &Value) -> bool {
        true
    }
    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    async fn call(
        &self,
        _input: &Value,
        _ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let sessions_dir = {
            let home = std::env::var("CLAUDE_CONFIG_DIR")
                .or_else(|_| std::env::var("HOME").map(|h| format!("{}/.claude", h)))
                .unwrap_or_else(|_| "/tmp/.claude".to_string());
            std::path::PathBuf::from(home).join("sessions")
        };
        let mut peers: Vec<Value> = Vec::new();
        let mut entries = match tokio::fs::read_dir(&sessions_dir).await {
            Ok(e) => e,
            Err(_) => {
                return Ok(ToolResultData {
                    data: json!({ "peers": [], "count": 0 }),
                    is_error: false,
                })
            }
        };
        let my_pid = std::process::id();
        while let Ok(Some(entry)) = entries.next_entry().await {
            let file_name = entry.file_name().to_string_lossy().to_string();
            if !file_name.ends_with(".json") {
                continue;
            }
            let pid: u32 = match file_name[..file_name.len() - 5].parse() {
                Ok(p) => p,
                Err(_) => continue,
            };
            if pid == my_pid {
                continue;
            }
            let alive = std::process::Command::new("kill")
                .args(["-0", &pid.to_string()])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if !alive {
                let _ = tokio::fs::remove_file(entry.path()).await;
                continue;
            }
            if let Ok(content) = tokio::fs::read_to_string(entry.path()).await {
                if let Ok(data) = serde_json::from_str::<Value>(&content) {
                    peers.push(json!({ "pid": pid, "sessionId": data.get("sessionId"), "cwd": data.get("cwd"), "startedAt": data.get("startedAt"), "name": data.get("name"), "kind": data.get("kind").unwrap_or(&json!("interactive")) }));
                }
            }
        }
        let count = peers.len();
        Ok(ToolResultData {
            data: json!({ "peers": peers, "count": count }),
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
        ToolUseContext::for_test(PathBuf::from("/tmp"), Arc::new(std::sync::Mutex::new(ReadFileState::new())), crate::registry::PermissionMode::Default)
    }
    #[tokio::test]
    async fn list_peers_returns_empty_when_no_sessions_dir() {
        std::env::set_var("CLAUDE_CONFIG_DIR", "/tmp/nonexistent_sessions_test_dir");
        let r = ListPeersTool
            .call(&json!({}), &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();
        assert!(!r.is_error);
        assert_eq!(r.data["count"].as_u64().unwrap(), 0);
        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }
    #[tokio::test]
    async fn list_peers_tool_properties() {
        assert_eq!(ListPeersTool.name(), "ListPeers");
        assert!(ListPeersTool.is_read_only(&json!({})));
        assert!(ListPeersTool.is_concurrency_safe(&json!({})));
    }
}
