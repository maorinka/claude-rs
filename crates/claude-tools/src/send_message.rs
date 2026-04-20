use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};
use uuid::Uuid;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

/// Verbatim port of TS SendMessageTool/prompt.ts `getPrompt()` with
/// UDS_INBOX branches inlined (Rust port always has inbox routing
/// support).
pub const SEND_MESSAGE_PROMPT: &str = include_str!("prompts/send_message.md");

// ---------------------------------------------------------------------------
// Mailbox data types
// ---------------------------------------------------------------------------

/// A message stored in an agent's file-based mailbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MailboxMessage {
    /// Unique ID for this message.
    pub id: String,
    /// Sender agent ID / name.
    pub from: String,
    /// Recipient agent ID / name.
    pub to: String,
    /// Optional human-readable summary (5-10 words).
    pub summary: Option<String>,
    /// Full message content.
    pub content: String,
    /// ISO-8601 timestamp of when the message was sent.
    pub timestamp: String,
}

// ---------------------------------------------------------------------------
// Mailbox helpers
// ---------------------------------------------------------------------------

/// Cross-platform home-directory resolution without an extra dependency.
fn home_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))
}

/// Returns `<home>/.claude/mailboxes/<agent_id>/`.
fn mailbox_dir_with_home(home: &PathBuf, agent_id: &str) -> PathBuf {
    home.join(".claude").join("mailboxes").join(agent_id)
}

/// Write a message to `~/.claude/mailboxes/<to>/msg_<uuid>.json`.
///
/// Creates the mailbox directory if it does not exist.
pub async fn write_to_mailbox(msg: &MailboxMessage) -> Result<PathBuf> {
    write_to_mailbox_in(&home_dir()?, msg).await
}

/// Internal write that accepts an explicit home root (for testing).
async fn write_to_mailbox_in(home: &PathBuf, msg: &MailboxMessage) -> Result<PathBuf> {
    let dir = mailbox_dir_with_home(home, &msg.to);
    tokio::fs::create_dir_all(&dir).await?;
    let file_name = format!("msg_{}.json", msg.id);
    let path = dir.join(&file_name);
    let json = serde_json::to_string_pretty(msg)?;
    tokio::fs::write(&path, json).await?;
    debug!(
        from = msg.from.as_str(),
        to = msg.to.as_str(),
        id = msg.id.as_str(),
        "Wrote message to mailbox"
    );
    Ok(path)
}

/// Read **and delete** all messages from `~/.claude/mailboxes/<agent_id>/`.
///
/// Files are processed in alphabetical order (which also gives a stable order
/// within the same delivery batch). Any file that cannot be parsed is skipped
/// with a warning.
pub async fn receive_messages(agent_id: &str) -> Result<Vec<MailboxMessage>> {
    receive_messages_in(&home_dir()?, agent_id).await
}

/// Internal receive that accepts an explicit home root (for testing).
async fn receive_messages_in(home: &PathBuf, agent_id: &str) -> Result<Vec<MailboxMessage>> {
    let dir = mailbox_dir_with_home(home, agent_id);

    if !dir.exists() {
        return Ok(vec![]);
    }

    let mut entries = tokio::fs::read_dir(&dir).await?;
    let mut paths: Vec<PathBuf> = Vec::new();

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            paths.push(path);
        }
    }

    // Sort for deterministic ordering.
    paths.sort();

    let mut messages = Vec::with_capacity(paths.len());

    for path in paths {
        match tokio::fs::read_to_string(&path).await {
            Ok(content) => match serde_json::from_str::<MailboxMessage>(&content) {
                Ok(msg) => {
                    if let Err(e) = tokio::fs::remove_file(&path).await {
                        warn!("Failed to delete mailbox message {:?}: {}", path, e);
                    }
                    messages.push(msg);
                }
                Err(e) => {
                    warn!("Skipping unparseable mailbox file {:?}: {}", path, e);
                }
            },
            Err(e) => {
                warn!("Could not read mailbox file {:?}: {}", path, e);
            }
        }
    }

    Ok(messages)
}

// ---------------------------------------------------------------------------
// Mailbox polling
// ---------------------------------------------------------------------------

/// Channel type for delivering polled messages to the agent loop.
pub type MailboxReceiver = mpsc::Receiver<Vec<MailboxMessage>>;

/// Global handle to the active polling task, so it can be stopped.
static POLLING_HANDLE: Lazy<Mutex<Option<CancellationToken>>> = Lazy::new(|| Mutex::new(None));

/// Start a background task that polls the mailbox for the given agent_id
/// every `interval`. New messages are sent through the returned channel.
///
/// Only one polling task runs at a time. Calling this again replaces the
/// previous poller.
pub async fn start_mailbox_polling(agent_id: String, interval: Duration) -> MailboxReceiver {
    // Stop any existing poller
    stop_mailbox_polling().await;

    let (tx, rx) = mpsc::channel::<Vec<MailboxMessage>>(32);
    let cancel = CancellationToken::new();

    {
        let mut handle = POLLING_HANDLE.lock().await;
        *handle = Some(cancel.clone());
    }

    let cancel_clone = cancel.clone();
    tokio::spawn(async move {
        debug!(agent_id = agent_id.as_str(), "Mailbox polling started");

        loop {
            tokio::select! {
                _ = cancel_clone.cancelled() => {
                    debug!(agent_id = agent_id.as_str(), "Mailbox polling cancelled");
                    break;
                }
                _ = tokio::time::sleep(interval) => {
                    match receive_messages(&agent_id).await {
                        Ok(messages) if !messages.is_empty() => {
                            debug!(
                                agent_id = agent_id.as_str(),
                                count = messages.len(),
                                "Polled {} message(s) from mailbox",
                                messages.len()
                            );
                            if tx.send(messages).await.is_err() {
                                // Receiver dropped -- stop polling
                                debug!(agent_id = agent_id.as_str(), "Mailbox receiver dropped, stopping poll");
                                break;
                            }
                        }
                        Ok(_) => {
                            // No messages, continue
                        }
                        Err(e) => {
                            warn!(agent_id = agent_id.as_str(), "Mailbox poll error: {}", e);
                        }
                    }
                }
            }
        }
    });

    rx
}

/// Stop the background mailbox polling task.
pub async fn stop_mailbox_polling() {
    let mut handle = POLLING_HANDLE.lock().await;
    if let Some(cancel) = handle.take() {
        cancel.cancel();
    }
}

/// Check if mailbox polling is currently active.
pub async fn is_polling_active() -> bool {
    let handle = POLLING_HANDLE.lock().await;
    handle.is_some()
}

// ---------------------------------------------------------------------------
// SendMessageTool
// ---------------------------------------------------------------------------

pub struct SendMessageTool;

#[async_trait]
impl ToolExecutor for SendMessageTool {
    fn name(&self) -> &str {
        "SendMessage"
    }

    fn description(&self) -> String {
        SEND_MESSAGE_PROMPT.to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["to", "content"],
            "properties": {
                "to": {
                    "type": "string",
                    "description": "Recipient: teammate name, or \"*\" for broadcast to all teammates."
                },
                "content": {
                    "type": "string",
                    "description": "The message content to send."
                },
                "summary": {
                    "type": "string",
                    "description": "A 5-10 word summary shown as a preview in the UI (required when message is a string)."
                },
                "from": {
                    "type": "string",
                    "description": "Sender agent ID or name. Defaults to CLAUDE_CODE_AGENT_NAME env var, then \"agent\"."
                }
            }
        })
    }

    fn is_read_only(&self, _input: &Value) -> bool {
        false
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        // Each message lands in a uniquely-named file; concurrent sends are safe.
        true
    }

    async fn call(
        &self,
        input: &Value,
        _ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        // --- validate required fields ---
        let to = match input["to"].as_str() {
            Some(t) if !t.trim().is_empty() => t.trim().to_string(),
            _ => {
                return Ok(ToolResultData {
                    data: json!({ "error": "missing or empty required field: to" }),
                    is_error: true,
                });
            }
        };

        // Reject '@' in recipient -- matches TS validation
        if to.contains('@') {
            return Ok(ToolResultData {
                data: json!({
                    "error": "to must be a bare teammate name or \"*\" -- there is only one team per session"
                }),
                is_error: true,
            });
        }

        let content = match input["content"].as_str() {
            Some(c) => c.to_string(),
            None => {
                return Ok(ToolResultData {
                    data: json!({ "error": "missing required field: content" }),
                    is_error: true,
                });
            }
        };

        let summary = input
            .get("summary")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let from = input
            .get("from")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                std::env::var("CLAUDE_CODE_AGENT_NAME").unwrap_or_else(|_| "agent".to_string())
            });

        let message_id = Uuid::new_v4().to_string();

        let msg = MailboxMessage {
            id: message_id.clone(),
            from: from.clone(),
            to: to.clone(),
            summary: summary.clone(),
            content,
            timestamp: Utc::now().to_rfc3339(),
        };

        match write_to_mailbox(&msg).await {
            Ok(_) => Ok(ToolResultData {
                data: json!({
                    "success": true,
                    "message": format!("Message sent to {}'s inbox", to),
                    "to": to,
                    "message_id": message_id,
                    "routing": {
                        "sender": from,
                        "target": format!("@{}", to),
                        "summary": summary,
                    }
                }),
                is_error: false,
            }),
            Err(e) => Ok(ToolResultData {
                data: json!({
                    "error": format!(
                        "Failed to deliver message to mailbox for \"{}\": {}", to, e
                    )
                }),
                is_error: true,
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn mk_msg(id: &str, from: &str, to: &str, content: &str) -> MailboxMessage {
        MailboxMessage {
            id: id.to_string(),
            from: from.to_string(),
            to: to.to_string(),
            summary: None,
            content: content.to_string(),
            timestamp: Utc::now().to_rfc3339(),
        }
    }

    #[tokio::test]
    async fn test_send_writes_file_to_correct_mailbox() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().to_path_buf();

        let msg = MailboxMessage {
            id: "test-id-1".to_string(),
            from: "sender-agent".to_string(),
            to: "receiver-agent".to_string(),
            summary: Some("hello world".to_string()),
            content: "Hello from sender!".to_string(),
            timestamp: Utc::now().to_rfc3339(),
        };

        let path = write_to_mailbox_in(&home, &msg).await.unwrap();

        let expected_dir = mailbox_dir_with_home(&home, "receiver-agent");
        assert!(
            path.starts_with(&expected_dir),
            "path {:?} not under {:?}",
            path,
            expected_dir
        );
        assert!(path.exists(), "message file not found at {:?}", path);

        let filename = path.file_name().unwrap().to_string_lossy().to_string();
        assert_eq!(filename, "msg_test-id-1.json");

        let raw = std::fs::read_to_string(&path).unwrap();
        let parsed: MailboxMessage = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed.id, "test-id-1");
        assert_eq!(parsed.from, "sender-agent");
        assert_eq!(parsed.to, "receiver-agent");
        assert_eq!(parsed.content, "Hello from sender!");
    }

    #[tokio::test]
    async fn test_receive_reads_and_deletes_messages() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().to_path_buf();
        let agent_id = "consumer-agent";

        for i in 0..2u8 {
            let msg = mk_msg(
                &format!("recv-id-{}", i),
                "producer",
                agent_id,
                &format!("Message number {}", i),
            );
            write_to_mailbox_in(&home, &msg).await.unwrap();
        }

        let dir = mailbox_dir_with_home(&home, agent_id);
        let count_before = std::fs::read_dir(&dir).unwrap().count();
        assert_eq!(count_before, 2, "expected 2 message files before receive");

        let received = receive_messages_in(&home, agent_id).await.unwrap();
        assert_eq!(received.len(), 2, "expected 2 received messages");

        let count_after = std::fs::read_dir(&dir).unwrap().count();
        assert_eq!(count_after, 0, "expected 0 message files after receive");

        let ids: Vec<_> = received.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["recv-id-0", "recv-id-1"]);
    }

    #[tokio::test]
    async fn test_send_to_nonexistent_agent_creates_mailbox() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().to_path_buf();
        let agent_id = "brand-new-agent";

        let dir = mailbox_dir_with_home(&home, agent_id);
        assert!(!dir.exists(), "mailbox dir should not exist yet");

        let msg = mk_msg("create-test", "orchestrator", agent_id, "Wake up!");
        let result = write_to_mailbox_in(&home, &msg).await;
        assert!(
            result.is_ok(),
            "write_to_mailbox_in failed: {:?}",
            result.err()
        );

        assert!(dir.exists(), "mailbox dir was not created");
        let file = dir.join("msg_create-test.json");
        assert!(file.exists(), "message file was not created");
    }

    #[tokio::test]
    async fn test_receive_empty_mailbox() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().to_path_buf();
        let messages = receive_messages_in(&home, "nobody").await.unwrap();
        assert!(
            messages.is_empty(),
            "expected no messages for unknown agent"
        );
    }

    #[tokio::test]
    async fn test_tool_call_returns_success_with_routing() {
        use crate::registry::ReadFileState;
        use std::sync::{Arc, Mutex};

        let tmp = TempDir::new().unwrap();
        unsafe { std::env::set_var("HOME", tmp.path()) };

        let tool = SendMessageTool;
        let input = json!({
            "to": "tool-target",
            "content": "integration check",
            "from": "tool-sender",
        });

        let ctx = ToolUseContext {
            working_directory: PathBuf::from("/tmp"),
            read_file_state: Arc::new(Mutex::new(ReadFileState::new())),
            permission_mode: crate::registry::PermissionMode::Default,
            ..Default::default()
        };
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(
            !result.is_error,
            "expected no error, got: {:?}",
            result.data
        );
        assert_eq!(result.data["success"], true);
        assert_eq!(result.data["to"], "tool-target");
        assert!(
            result.data["message_id"].is_string(),
            "message_id should be a string UUID"
        );
        // Verify routing info
        assert_eq!(result.data["routing"]["sender"], "tool-sender");
        assert_eq!(result.data["routing"]["target"], "@tool-target");
    }

    #[tokio::test]
    async fn test_tool_call_error_missing_to() {
        use crate::registry::ReadFileState;
        use std::sync::{Arc, Mutex};

        let tool = SendMessageTool;
        let input = json!({ "content": "hello" });

        let ctx = ToolUseContext {
            working_directory: PathBuf::from("/tmp"),
            read_file_state: Arc::new(Mutex::new(ReadFileState::new())),
            permission_mode: crate::registry::PermissionMode::Default,
            ..Default::default()
        };
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(result.is_error);
        assert!(result.data["error"].as_str().unwrap().contains("to"));
    }

    #[tokio::test]
    async fn test_tool_rejects_at_sign_in_recipient() {
        use crate::registry::ReadFileState;
        use std::sync::{Arc, Mutex};

        let tool = SendMessageTool;
        let input = json!({ "to": "agent@team", "content": "hello" });

        let ctx = ToolUseContext {
            working_directory: PathBuf::from("/tmp"),
            read_file_state: Arc::new(Mutex::new(ReadFileState::new())),
            permission_mode: crate::registry::PermissionMode::Default,
            ..Default::default()
        };
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(result.is_error);
        assert!(
            result.data["error"]
                .as_str()
                .unwrap()
                .contains("bare teammate name"),
            "should reject @ in recipient"
        );
    }

    #[tokio::test]
    async fn test_polling_receive_roundtrip() {
        // Test the core polling logic: write, receive, verify consumed.
        // Uses the internal _in variants to avoid global HOME races.
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().to_path_buf();
        let agent_id = "poll-roundtrip";

        let msg = mk_msg("poll-1", "sender", agent_id, "Polled message");
        write_to_mailbox_in(&home, &msg).await.unwrap();

        let received = receive_messages_in(&home, agent_id).await.unwrap();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].id, "poll-1");
        assert_eq!(received[0].content, "Polled message");

        // After receive, mailbox should be empty (messages are consumed)
        let empty = receive_messages_in(&home, agent_id).await.unwrap();
        assert!(empty.is_empty(), "mailbox should be empty after receive");
    }

    #[tokio::test]
    async fn test_polling_multiple_messages_ordered() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().to_path_buf();
        let agent_id = "poll-ordered";

        for i in 0..3u8 {
            let msg = mk_msg(
                &format!("ord-{}", i),
                "sender",
                agent_id,
                &format!("Message {}", i),
            );
            write_to_mailbox_in(&home, &msg).await.unwrap();
        }

        let received = receive_messages_in(&home, agent_id).await.unwrap();
        assert_eq!(received.len(), 3);
        let ids: Vec<_> = received.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["ord-0", "ord-1", "ord-2"]);
    }

    #[tokio::test]
    async fn test_polling_start_stop_contract() {
        // Verify start creates a receiver and stop is idempotent
        // (no global assertions to avoid cross-test races)
        let rx = start_mailbox_polling("contract-test".to_string(), Duration::from_secs(60)).await;
        // rx is a valid receiver
        drop(rx);
        // stop is safe even without the receiver
        stop_mailbox_polling().await;
        // stop is idempotent
        stop_mailbox_polling().await;
    }
}
