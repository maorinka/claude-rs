//! Teammate Mailbox - File-based messaging system for agent swarms.
//!
//! Each teammate has an inbox file at `~/.claude/teams/{team_name}/inboxes/{agent_name}.json`.
//! Other teammates can write messages to it, and the recipient sees them as attachments.
//!
//! Inboxes are keyed by agent name within a team.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use super::types::BackendType;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Lock retry configuration: retry with backoff so concurrent callers
/// (multiple Claude instances in a swarm) wait for the lock instead of
/// failing immediately.
const LOCK_MAX_RETRIES: u32 = 10;
const LOCK_MIN_TIMEOUT_MS: u64 = 5;
const LOCK_MAX_TIMEOUT_MS: u64 = 100;

/// Default team name when none is specified.
const DEFAULT_TEAM: &str = "default";

/// XML tag used when formatting teammate messages for display.
const TEAMMATE_MESSAGE_TAG: &str = "teammate-message";

/// Team lead sentinel name.
pub const TEAM_LEAD_NAME: &str = "team-lead";

// ---------------------------------------------------------------------------
// TeammateMessage
// ---------------------------------------------------------------------------

/// A message in a teammate's inbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeammateMessage {
    pub from: String,
    pub text: String,
    pub timestamp: String,
    pub read: bool,
    /// Sender's assigned color (e.g. "red", "blue", "green").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// 5-10 word summary shown as preview in the UI.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

/// Return the base directory for teams: `~/.claude/teams/`.
fn teams_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude")
        .join("teams")
}

/// Sanitize a path component to prevent directory traversal.
fn sanitize_path_component(s: &str) -> String {
    s.replace('/', "_")
        .replace('\\', "_")
        .replace("..", "_")
        .replace('\0', "")
}

/// Get the path to a teammate's inbox file.
/// Structure: `~/.claude/teams/{team_name}/inboxes/{agent_name}.json`
pub fn get_inbox_path(agent_name: &str, team_name: Option<&str>) -> PathBuf {
    let team = team_name.unwrap_or(DEFAULT_TEAM);
    let safe_team = sanitize_path_component(team);
    let safe_agent = sanitize_path_component(agent_name);
    let inbox_dir = teams_dir().join(&safe_team).join("inboxes");
    inbox_dir.join(format!("{}.json", safe_agent))
}

/// Get the inbox directory for a team.
fn get_inbox_dir(team_name: Option<&str>) -> PathBuf {
    let team = team_name.unwrap_or(DEFAULT_TEAM);
    let safe_team = sanitize_path_component(team);
    teams_dir().join(&safe_team).join("inboxes")
}

/// Ensure the inbox directory exists for a team.
async fn ensure_inbox_dir(team_name: Option<&str>) -> Result<()> {
    let dir = get_inbox_dir(team_name);
    tokio::fs::create_dir_all(&dir)
        .await
        .with_context(|| format!("create inbox dir {:?}", dir))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// File locking with retry
// ---------------------------------------------------------------------------

/// Simple file-lock via a `.lock` file with retry.
/// Returns a guard that removes the lock on drop.
async fn acquire_lock(lock_path: &Path) -> Result<LockGuard> {
    let mut retry = 0;
    loop {
        match tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(lock_path)
            .await
        {
            Ok(_) => {
                return Ok(LockGuard {
                    path: lock_path.to_path_buf(),
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if retry >= LOCK_MAX_RETRIES {
                    // Try to clean up a potentially stale lock.
                    let _ = tokio::fs::remove_file(lock_path).await;
                    return Err(anyhow::anyhow!(
                        "failed to acquire lock {:?} after {} retries",
                        lock_path,
                        LOCK_MAX_RETRIES
                    ));
                }
                let delay = std::cmp::min(
                    LOCK_MIN_TIMEOUT_MS * (1 << retry.min(6)),
                    LOCK_MAX_TIMEOUT_MS,
                );
                tokio::time::sleep(Duration::from_millis(delay)).await;
                retry += 1;
            }
            Err(e) => {
                return Err(anyhow::anyhow!("lock open error for {:?}: {}", lock_path, e));
            }
        }
    }
}

/// RAII guard that removes the lock file on drop.
struct LockGuard {
    path: PathBuf,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

// ---------------------------------------------------------------------------
// Core mailbox operations
// ---------------------------------------------------------------------------

/// Read all messages from a teammate's inbox.
pub async fn read_mailbox(
    agent_name: &str,
    team_name: Option<&str>,
) -> Vec<TeammateMessage> {
    let inbox_path = get_inbox_path(agent_name, team_name);
    debug!("[TeammateMailbox] readMailbox: path={:?}", inbox_path);

    match tokio::fs::read_to_string(&inbox_path).await {
        Ok(content) => match serde_json::from_str::<Vec<TeammateMessage>>(&content) {
            Ok(messages) => {
                debug!(
                    "[TeammateMailbox] readMailbox: read {} message(s)",
                    messages.len()
                );
                messages
            }
            Err(e) => {
                warn!("[TeammateMailbox] parse error for {:?}: {}", inbox_path, e);
                Vec::new()
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            debug!("[TeammateMailbox] readMailbox: file does not exist");
            Vec::new()
        }
        Err(e) => {
            warn!("Failed to read inbox for {}: {}", agent_name, e);
            Vec::new()
        }
    }
}

/// Read only unread messages from a teammate's inbox.
pub async fn read_unread_messages(
    agent_name: &str,
    team_name: Option<&str>,
) -> Vec<TeammateMessage> {
    let messages = read_mailbox(agent_name, team_name).await;
    let unread: Vec<TeammateMessage> = messages.into_iter().filter(|m| !m.read).collect();
    debug!(
        "[TeammateMailbox] readUnreadMessages: {} unread",
        unread.len()
    );
    unread
}

/// Write a message to a teammate's inbox.
/// Uses file locking to prevent race conditions when multiple agents write concurrently.
pub async fn write_to_mailbox(
    recipient_name: &str,
    message: TeammateMessageInput,
    team_name: Option<&str>,
) -> Result<()> {
    ensure_inbox_dir(team_name).await?;

    let inbox_path = get_inbox_path(recipient_name, team_name);
    let lock_path = inbox_path.with_extension("json.lock");

    debug!(
        "[TeammateMailbox] writeToMailbox: recipient={}, from={}, path={:?}",
        recipient_name, message.from, inbox_path
    );

    let _guard = acquire_lock(&lock_path).await?;

    // Ensure the inbox file exists *after* acquiring the lock to avoid a
    // TOCTOU race between the existence check and the write.
    if !inbox_path.exists() {
        match tokio::fs::write(&inbox_path, "[]").await {
            Ok(_) => debug!("[TeammateMailbox] writeToMailbox: created new inbox file"),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => {
                warn!("[TeammateMailbox] writeToMailbox: failed to create inbox file: {}", e);
                return Err(e.into());
            }
        }
    }

    // Re-read messages after acquiring lock to get the latest state.
    let mut messages = read_mailbox(recipient_name, team_name).await;

    let new_message = TeammateMessage {
        from: message.from,
        text: message.text,
        timestamp: message.timestamp,
        read: false,
        color: message.color,
        summary: message.summary,
    };
    messages.push(new_message);

    let json = serde_json::to_string_pretty(&messages)?;
    tokio::fs::write(&inbox_path, json)
        .await
        .with_context(|| format!("write inbox {:?}", inbox_path))?;

    debug!(
        "[TeammateMailbox] Wrote message to {}'s inbox",
        recipient_name
    );
    Ok(())
}

/// Input for `write_to_mailbox` -- message without the `read` field.
#[derive(Debug, Clone)]
pub struct TeammateMessageInput {
    pub from: String,
    pub text: String,
    pub timestamp: String,
    pub color: Option<String>,
    pub summary: Option<String>,
}

/// Mark a specific message in a teammate's inbox as read by index.
pub async fn mark_message_as_read_by_index(
    agent_name: &str,
    team_name: Option<&str>,
    message_index: usize,
) -> Result<()> {
    let inbox_path = get_inbox_path(agent_name, team_name);
    let lock_path = inbox_path.with_extension("json.lock");

    let _guard = acquire_lock(&lock_path).await?;

    let mut messages = read_mailbox(agent_name, team_name).await;

    if message_index >= messages.len() {
        debug!(
            "[TeammateMailbox] markMessageAsReadByIndex: index {} out of bounds ({})",
            message_index,
            messages.len()
        );
        return Ok(());
    }

    if let Some(msg) = messages.get_mut(message_index) {
        if msg.read {
            return Ok(());
        }
        msg.read = true;
    }

    let json = serde_json::to_string_pretty(&messages)?;
    tokio::fs::write(&inbox_path, json).await?;
    debug!(
        "[TeammateMailbox] markMessageAsReadByIndex: marked message at index {} as read",
        message_index
    );
    Ok(())
}

/// Mark all messages in a teammate's inbox as read.
pub async fn mark_messages_as_read(
    agent_name: &str,
    team_name: Option<&str>,
) -> Result<()> {
    let inbox_path = get_inbox_path(agent_name, team_name);
    if !inbox_path.exists() {
        return Ok(());
    }
    let lock_path = inbox_path.with_extension("json.lock");

    let _guard = acquire_lock(&lock_path).await?;

    let mut messages = read_mailbox(agent_name, team_name).await;
    if messages.is_empty() {
        return Ok(());
    }

    let unread_count = messages.iter().filter(|m| !m.read).count();
    for m in &mut messages {
        m.read = true;
    }

    let json = serde_json::to_string_pretty(&messages)?;
    tokio::fs::write(&inbox_path, json).await?;
    debug!(
        "[TeammateMailbox] markMessagesAsRead: WROTE {} message(s) as read",
        unread_count
    );
    Ok(())
}

/// Mark messages matching a predicate as read, leaving others unread.
pub async fn mark_messages_as_read_by_predicate(
    agent_name: &str,
    team_name: Option<&str>,
    predicate: impl Fn(&TeammateMessage) -> bool,
) -> Result<()> {
    let inbox_path = get_inbox_path(agent_name, team_name);
    if !inbox_path.exists() {
        return Ok(());
    }
    let lock_path = inbox_path.with_extension("json.lock");

    let _guard = acquire_lock(&lock_path).await?;

    let messages = read_mailbox(agent_name, team_name).await;
    if messages.is_empty() {
        return Ok(());
    }

    let updated: Vec<TeammateMessage> = messages
        .into_iter()
        .map(|m| {
            if !m.read && predicate(&m) {
                TeammateMessage { read: true, ..m }
            } else {
                m
            }
        })
        .collect();

    let json = serde_json::to_string_pretty(&updated)?;
    tokio::fs::write(&inbox_path, json).await?;
    Ok(())
}

/// Clear a teammate's inbox (delete all messages).
pub async fn clear_mailbox(
    agent_name: &str,
    team_name: Option<&str>,
) -> Result<()> {
    let inbox_path = get_inbox_path(agent_name, team_name);
    match tokio::fs::write(&inbox_path, "[]").await {
        Ok(_) => {
            debug!("[TeammateMailbox] Cleared inbox for {}", agent_name);
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => {
            warn!("Failed to clear inbox for {}: {}", agent_name, e);
            Err(e.into())
        }
    }
}

// ---------------------------------------------------------------------------
// Format messages as XML
// ---------------------------------------------------------------------------

/// Format teammate messages as XML for attachment display.
pub fn format_teammate_messages(
    messages: &[TeammateMessage],
) -> String {
    messages
        .iter()
        .map(|m| {
            let color_attr = m.color.as_ref().map_or(String::new(), |c| format!(" color=\"{}\"", c));
            let summary_attr = m.summary.as_ref().map_or(String::new(), |s| format!(" summary=\"{}\"", s));
            format!(
                "<{tag} teammate_id=\"{from}\"{color}{summary}>\n{text}\n</{tag}>",
                tag = TEAMMATE_MESSAGE_TAG,
                from = m.from,
                color = color_attr,
                summary = summary_attr,
                text = m.text,
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

// ===========================================================================
// Structured message types (all 10+ protocol messages)
// ===========================================================================

// ---------------------------------------------------------------------------
// 1. IdleNotification
// ---------------------------------------------------------------------------

/// Structured message sent when a teammate becomes idle (via Stop hook).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdleNotificationMessage {
    #[serde(rename = "type")]
    pub msg_type: String, // always "idle_notification"
    pub from: String,
    pub timestamp: String,
    /// Why the agent went idle.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idle_reason: Option<IdleReason>,
    /// Brief summary of the last DM sent this turn (if any).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_status: Option<CompletedStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IdleReason {
    Available,
    Interrupted,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CompletedStatus {
    Resolved,
    Blocked,
    Failed,
}

/// Creates an idle notification message to send to the team leader.
pub fn create_idle_notification(
    agent_id: &str,
    idle_reason: Option<IdleReason>,
    summary: Option<String>,
    completed_task_id: Option<String>,
    completed_status: Option<CompletedStatus>,
    failure_reason: Option<String>,
) -> IdleNotificationMessage {
    IdleNotificationMessage {
        msg_type: "idle_notification".to_string(),
        from: agent_id.to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        idle_reason,
        summary,
        completed_task_id,
        completed_status,
        failure_reason,
    }
}

/// Checks if a message text contains an idle notification.
pub fn is_idle_notification(text: &str) -> Option<IdleNotificationMessage> {
    let parsed: serde_json::Value = serde_json::from_str(text).ok()?;
    if parsed.get("type")?.as_str()? == "idle_notification" {
        serde_json::from_value(parsed).ok()
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// 2. PermissionRequest
// ---------------------------------------------------------------------------

/// Permission request message sent from worker to leader via mailbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRequestMessage {
    #[serde(rename = "type")]
    pub msg_type: String, // always "permission_request"
    pub request_id: String,
    pub agent_id: String,
    pub tool_name: String,
    pub tool_use_id: String,
    pub description: String,
    pub input: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub permission_suggestions: Vec<serde_json::Value>,
}

/// Creates a permission request message.
pub fn create_permission_request_message(
    request_id: &str,
    agent_id: &str,
    tool_name: &str,
    tool_use_id: &str,
    description: &str,
    input: HashMap<String, serde_json::Value>,
    permission_suggestions: Vec<serde_json::Value>,
) -> PermissionRequestMessage {
    PermissionRequestMessage {
        msg_type: "permission_request".to_string(),
        request_id: request_id.to_string(),
        agent_id: agent_id.to_string(),
        tool_name: tool_name.to_string(),
        tool_use_id: tool_use_id.to_string(),
        description: description.to_string(),
        input,
        permission_suggestions,
    }
}

/// Checks if a message text contains a permission request.
pub fn is_permission_request(text: &str) -> Option<PermissionRequestMessage> {
    let parsed: serde_json::Value = serde_json::from_str(text).ok()?;
    if parsed.get("type")?.as_str()? == "permission_request" {
        serde_json::from_value(parsed).ok()
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// 3. PermissionResponse
// ---------------------------------------------------------------------------

/// Permission response message sent from leader to worker via mailbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "subtype")]
pub enum PermissionResponseMessage {
    #[serde(rename = "success")]
    Success {
        #[serde(rename = "type")]
        msg_type: String, // always "permission_response"
        request_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        response: Option<PermissionResponseData>,
    },
    #[serde(rename = "error")]
    Error {
        #[serde(rename = "type")]
        msg_type: String, // always "permission_response"
        request_id: String,
        error: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionResponseData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_input: Option<HashMap<String, serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_updates: Option<Vec<serde_json::Value>>,
}

/// Creates a permission response message (success variant).
pub fn create_permission_response_success(
    request_id: &str,
    updated_input: Option<HashMap<String, serde_json::Value>>,
    permission_updates: Option<Vec<serde_json::Value>>,
) -> PermissionResponseMessage {
    PermissionResponseMessage::Success {
        msg_type: "permission_response".to_string(),
        request_id: request_id.to_string(),
        response: Some(PermissionResponseData {
            updated_input,
            permission_updates,
        }),
    }
}

/// Creates a permission response message (error variant).
pub fn create_permission_response_error(
    request_id: &str,
    error: &str,
) -> PermissionResponseMessage {
    PermissionResponseMessage::Error {
        msg_type: "permission_response".to_string(),
        request_id: request_id.to_string(),
        error: error.to_string(),
    }
}

/// Checks if a message text contains a permission response.
pub fn is_permission_response(text: &str) -> Option<serde_json::Value> {
    let parsed: serde_json::Value = serde_json::from_str(text).ok()?;
    if parsed.get("type")?.as_str()? == "permission_response" {
        Some(parsed)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// 4. SandboxPermissionRequest
// ---------------------------------------------------------------------------

/// Sandbox permission request from worker to leader via mailbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxPermissionRequestMessage {
    #[serde(rename = "type")]
    pub msg_type: String, // always "sandbox_permission_request"
    pub request_id: String,
    pub worker_id: String,
    pub worker_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worker_color: Option<String>,
    pub host_pattern: HostPattern,
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostPattern {
    pub host: String,
}

/// Creates a sandbox permission request message.
pub fn create_sandbox_permission_request(
    request_id: &str,
    worker_id: &str,
    worker_name: &str,
    worker_color: Option<&str>,
    host: &str,
) -> SandboxPermissionRequestMessage {
    SandboxPermissionRequestMessage {
        msg_type: "sandbox_permission_request".to_string(),
        request_id: request_id.to_string(),
        worker_id: worker_id.to_string(),
        worker_name: worker_name.to_string(),
        worker_color: worker_color.map(|s| s.to_string()),
        host_pattern: HostPattern {
            host: host.to_string(),
        },
        created_at: chrono::Utc::now().timestamp_millis() as u64,
    }
}

/// Checks if a message text contains a sandbox permission request.
pub fn is_sandbox_permission_request(text: &str) -> Option<SandboxPermissionRequestMessage> {
    let parsed: serde_json::Value = serde_json::from_str(text).ok()?;
    if parsed.get("type")?.as_str()? == "sandbox_permission_request" {
        serde_json::from_value(parsed).ok()
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// 5. SandboxPermissionResponse
// ---------------------------------------------------------------------------

/// Sandbox permission response from leader to worker via mailbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxPermissionResponseMessage {
    #[serde(rename = "type")]
    pub msg_type: String, // always "sandbox_permission_response"
    pub request_id: String,
    pub host: String,
    pub allow: bool,
    pub timestamp: String,
}

/// Creates a sandbox permission response message.
pub fn create_sandbox_permission_response(
    request_id: &str,
    host: &str,
    allow: bool,
) -> SandboxPermissionResponseMessage {
    SandboxPermissionResponseMessage {
        msg_type: "sandbox_permission_response".to_string(),
        request_id: request_id.to_string(),
        host: host.to_string(),
        allow,
        timestamp: chrono::Utc::now().to_rfc3339(),
    }
}

/// Checks if a message text contains a sandbox permission response.
pub fn is_sandbox_permission_response(text: &str) -> Option<SandboxPermissionResponseMessage> {
    let parsed: serde_json::Value = serde_json::from_str(text).ok()?;
    if parsed.get("type")?.as_str()? == "sandbox_permission_response" {
        serde_json::from_value(parsed).ok()
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// 6. PlanApprovalRequest
// ---------------------------------------------------------------------------

/// Message sent when a teammate requests plan approval from the team leader.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanApprovalRequestMessage {
    #[serde(rename = "type")]
    pub msg_type: String, // always "plan_approval_request"
    pub from: String,
    pub timestamp: String,
    pub plan_file_path: String,
    pub plan_content: String,
    pub request_id: String,
}

/// Creates a plan approval request message.
pub fn create_plan_approval_request(
    from: &str,
    plan_file_path: &str,
    plan_content: &str,
    request_id: &str,
) -> PlanApprovalRequestMessage {
    PlanApprovalRequestMessage {
        msg_type: "plan_approval_request".to_string(),
        from: from.to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        plan_file_path: plan_file_path.to_string(),
        plan_content: plan_content.to_string(),
        request_id: request_id.to_string(),
    }
}

/// Checks if a message text contains a plan approval request.
pub fn is_plan_approval_request(text: &str) -> Option<PlanApprovalRequestMessage> {
    let parsed: serde_json::Value = serde_json::from_str(text).ok()?;
    if parsed.get("type")?.as_str()? == "plan_approval_request" {
        serde_json::from_value(parsed).ok()
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// 7. PlanApprovalResponse
// ---------------------------------------------------------------------------

/// Response from leader to teammate plan approval request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanApprovalResponseMessage {
    #[serde(rename = "type")]
    pub msg_type: String, // always "plan_approval_response"
    pub request_id: String,
    pub approved: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feedback: Option<String>,
    pub timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
}

/// Creates a plan approval response message.
pub fn create_plan_approval_response(
    request_id: &str,
    approved: bool,
    feedback: Option<&str>,
    permission_mode: Option<&str>,
) -> PlanApprovalResponseMessage {
    PlanApprovalResponseMessage {
        msg_type: "plan_approval_response".to_string(),
        request_id: request_id.to_string(),
        approved,
        feedback: feedback.map(|s| s.to_string()),
        timestamp: chrono::Utc::now().to_rfc3339(),
        permission_mode: permission_mode.map(|s| s.to_string()),
    }
}

/// Checks if a message text contains a plan approval response.
pub fn is_plan_approval_response(text: &str) -> Option<PlanApprovalResponseMessage> {
    let parsed: serde_json::Value = serde_json::from_str(text).ok()?;
    if parsed.get("type")?.as_str()? == "plan_approval_response" {
        serde_json::from_value(parsed).ok()
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// 8. ShutdownRequest
// ---------------------------------------------------------------------------

/// Shutdown request message sent from leader to teammate via mailbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShutdownRequestMessage {
    #[serde(rename = "type")]
    pub msg_type: String, // always "shutdown_request"
    pub request_id: String,
    pub from: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub timestamp: String,
}

/// Creates a shutdown request message.
pub fn create_shutdown_request(
    request_id: &str,
    from: &str,
    reason: Option<&str>,
) -> ShutdownRequestMessage {
    ShutdownRequestMessage {
        msg_type: "shutdown_request".to_string(),
        request_id: request_id.to_string(),
        from: from.to_string(),
        reason: reason.map(|s| s.to_string()),
        timestamp: chrono::Utc::now().to_rfc3339(),
    }
}

/// Checks if a message text contains a shutdown request.
pub fn is_shutdown_request(text: &str) -> Option<ShutdownRequestMessage> {
    let parsed: serde_json::Value = serde_json::from_str(text).ok()?;
    if parsed.get("type")?.as_str()? == "shutdown_request" {
        serde_json::from_value(parsed).ok()
    } else {
        None
    }
}

/// Sends a shutdown request to a teammate's mailbox.
pub async fn send_shutdown_request_to_mailbox(
    target_name: &str,
    team_name: Option<&str>,
    reason: Option<&str>,
    sender_name: Option<&str>,
    sender_color: Option<&str>,
) -> Result<(String, String)> {
    let sender = sender_name.unwrap_or(TEAM_LEAD_NAME);

    // Generate a deterministic request ID for this shutdown request.
    let request_id = generate_request_id("shutdown", target_name);

    let shutdown_message = create_shutdown_request(&request_id, sender, reason);

    write_to_mailbox(
        target_name,
        TeammateMessageInput {
            from: sender.to_string(),
            text: serde_json::to_string(&shutdown_message)?,
            timestamp: chrono::Utc::now().to_rfc3339(),
            color: sender_color.map(|s| s.to_string()),
            summary: None,
        },
        team_name,
    )
    .await?;

    Ok((request_id, target_name.to_string()))
}

// ---------------------------------------------------------------------------
// 9. ShutdownApproved
// ---------------------------------------------------------------------------

/// Shutdown approved message sent from teammate to leader via mailbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShutdownApprovedMessage {
    #[serde(rename = "type")]
    pub msg_type: String, // always "shutdown_approved"
    pub request_id: String,
    pub from: String,
    pub timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend_type: Option<String>,
}

/// Creates a shutdown approved message.
pub fn create_shutdown_approved(
    request_id: &str,
    from: &str,
    pane_id: Option<&str>,
    backend_type: Option<BackendType>,
) -> ShutdownApprovedMessage {
    ShutdownApprovedMessage {
        msg_type: "shutdown_approved".to_string(),
        request_id: request_id.to_string(),
        from: from.to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        pane_id: pane_id.map(|s| s.to_string()),
        backend_type: backend_type.map(|b| b.to_string()),
    }
}

/// Checks if a message text contains a shutdown approved message.
pub fn is_shutdown_approved(text: &str) -> Option<ShutdownApprovedMessage> {
    let parsed: serde_json::Value = serde_json::from_str(text).ok()?;
    if parsed.get("type")?.as_str()? == "shutdown_approved" {
        serde_json::from_value(parsed).ok()
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// 10. ShutdownRejected
// ---------------------------------------------------------------------------

/// Shutdown rejected message sent from teammate to leader via mailbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShutdownRejectedMessage {
    #[serde(rename = "type")]
    pub msg_type: String, // always "shutdown_rejected"
    pub request_id: String,
    pub from: String,
    pub reason: String,
    pub timestamp: String,
}

/// Creates a shutdown rejected message.
pub fn create_shutdown_rejected(
    request_id: &str,
    from: &str,
    reason: &str,
) -> ShutdownRejectedMessage {
    ShutdownRejectedMessage {
        msg_type: "shutdown_rejected".to_string(),
        request_id: request_id.to_string(),
        from: from.to_string(),
        reason: reason.to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
    }
}

/// Checks if a message text contains a shutdown rejected message.
pub fn is_shutdown_rejected(text: &str) -> Option<ShutdownRejectedMessage> {
    let parsed: serde_json::Value = serde_json::from_str(text).ok()?;
    if parsed.get("type")?.as_str()? == "shutdown_rejected" {
        serde_json::from_value(parsed).ok()
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// 11. ModeSetRequest
// ---------------------------------------------------------------------------

/// Mode set request message sent from leader to teammate via mailbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModeSetRequestMessage {
    #[serde(rename = "type")]
    pub msg_type: String, // always "mode_set_request"
    pub mode: String,
    pub from: String,
}

/// Creates a mode set request message.
pub fn create_mode_set_request(mode: &str, from: &str) -> ModeSetRequestMessage {
    ModeSetRequestMessage {
        msg_type: "mode_set_request".to_string(),
        mode: mode.to_string(),
        from: from.to_string(),
    }
}

/// Checks if a message text contains a mode set request.
pub fn is_mode_set_request(text: &str) -> Option<ModeSetRequestMessage> {
    let parsed: serde_json::Value = serde_json::from_str(text).ok()?;
    if parsed.get("type")?.as_str()? == "mode_set_request" {
        serde_json::from_value(parsed).ok()
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// 12. TeamPermissionUpdate (broadcast)
// ---------------------------------------------------------------------------

/// Team permission update message sent from leader to teammates via mailbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamPermissionUpdateMessage {
    #[serde(rename = "type")]
    pub msg_type: String, // always "team_permission_update"
    pub permission_update: PermissionUpdatePayload,
    pub directory_path: String,
    pub tool_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionUpdatePayload {
    #[serde(rename = "type")]
    pub update_type: String, // "addRules"
    pub rules: Vec<PermissionRule>,
    pub behavior: String, // "allow" | "deny" | "ask"
    pub destination: String, // "session"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionRule {
    pub tool_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_content: Option<String>,
}

/// Checks if a message text contains a team permission update.
pub fn is_team_permission_update(text: &str) -> Option<TeamPermissionUpdateMessage> {
    let parsed: serde_json::Value = serde_json::from_str(text).ok()?;
    if parsed.get("type")?.as_str()? == "team_permission_update" {
        serde_json::from_value(parsed).ok()
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// 13. TaskAssignment
// ---------------------------------------------------------------------------

/// Task assignment message sent when a task is assigned to a teammate.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskAssignmentMessage {
    #[serde(rename = "type")]
    pub msg_type: String, // always "task_assignment"
    pub task_id: String,
    pub subject: String,
    pub description: String,
    pub assigned_by: String,
    pub timestamp: String,
}

/// Checks if a message text contains a task assignment.
pub fn is_task_assignment(text: &str) -> Option<TaskAssignmentMessage> {
    let parsed: serde_json::Value = serde_json::from_str(text).ok()?;
    if parsed.get("type")?.as_str()? == "task_assignment" {
        serde_json::from_value(parsed).ok()
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Structured protocol message detection
// ---------------------------------------------------------------------------

/// Protocol message types that should be routed by inbox poller rather than
/// consumed as raw LLM context.
const STRUCTURED_PROTOCOL_TYPES: &[&str] = &[
    "permission_request",
    "permission_response",
    "sandbox_permission_request",
    "sandbox_permission_response",
    "shutdown_request",
    "shutdown_approved",
    "shutdown_rejected",
    "idle_notification",
    "task_assignment",
    "team_permission_update",
    "mode_set_request",
    "plan_approval_request",
    "plan_approval_response",
];

/// Checks if a message text is a structured protocol message that should be
/// routed by the inbox poller rather than consumed as raw LLM context.
pub fn is_structured_protocol_message(text: &str) -> bool {
    let parsed: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => return false,
    };

    match parsed.get("type").and_then(|v| v.as_str()) {
        Some(t) => STRUCTURED_PROTOCOL_TYPES.contains(&t),
        None => false,
    }
}

// ---------------------------------------------------------------------------
// Request ID generation
// ---------------------------------------------------------------------------

/// Generate a request ID with a prefix and agent name.
pub fn generate_request_id(prefix: &str, agent_name: &str) -> String {
    let ts = chrono::Utc::now().timestamp_millis();
    let rand_part: String = (0..7)
        .map(|_| {
            let idx = rand::random::<u8>() % 36;
            if idx < 10 {
                (b'0' + idx) as char
            } else {
                (b'a' + idx - 10) as char
            }
        })
        .collect();
    format!("{}-{}-{}-{}", prefix, agent_name, ts, rand_part)
}

// ---------------------------------------------------------------------------
// Helpers for reading team files
// ---------------------------------------------------------------------------

use super::types::TeamFile;

/// Get the team directory path.
pub fn get_team_dir(team_name: &str) -> PathBuf {
    let safe = sanitize_path_component(team_name);
    teams_dir().join(safe)
}

/// Read the team file async.
pub async fn read_team_file(team_name: &str) -> Option<TeamFile> {
    let path = get_team_dir(team_name).join("team.json");
    let content = tokio::fs::read_to_string(&path).await.ok()?;
    serde_json::from_str(&content).ok()
}

/// Write the team file async.
pub async fn write_team_file(team_name: &str, team_file: &TeamFile) -> Result<()> {
    let dir = get_team_dir(team_name);
    tokio::fs::create_dir_all(&dir).await?;
    let path = dir.join("team.json");
    let json = serde_json::to_string_pretty(team_file)?;
    tokio::fs::write(&path, json).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_structured_protocol_message() {
        assert!(is_structured_protocol_message(
            r#"{"type": "permission_request", "request_id": "r1"}"#
        ));
        assert!(is_structured_protocol_message(
            r#"{"type": "shutdown_request", "requestId": "s1"}"#
        ));
        assert!(is_structured_protocol_message(
            r#"{"type": "mode_set_request", "mode": "auto", "from": "lead"}"#
        ));
        assert!(!is_structured_protocol_message("just some text"));
        assert!(!is_structured_protocol_message(
            r#"{"type": "unknown_type"}"#
        ));
        assert!(!is_structured_protocol_message(""));
    }

    #[test]
    fn test_idle_notification_roundtrip() {
        let msg = create_idle_notification(
            "worker-1",
            Some(IdleReason::Available),
            Some("Finished research".to_string()),
            None,
            None,
            None,
        );
        let json = serde_json::to_string(&msg).unwrap();
        let parsed = is_idle_notification(&json);
        assert!(parsed.is_some());
        let parsed = parsed.unwrap();
        assert_eq!(parsed.from, "worker-1");
        assert_eq!(parsed.idle_reason, Some(IdleReason::Available));
    }

    #[test]
    fn test_permission_request_roundtrip() {
        let msg = create_permission_request_message(
            "req-1",
            "worker-1",
            "Bash",
            "tu-1",
            "Run npm test",
            HashMap::new(),
            vec![],
        );
        let json = serde_json::to_string(&msg).unwrap();
        let parsed = is_permission_request(&json);
        assert!(parsed.is_some());
        assert_eq!(parsed.unwrap().tool_name, "Bash");
    }

    #[test]
    fn test_shutdown_messages() {
        let req = create_shutdown_request("sr-1", "team-lead", Some("cleaning up"));
        let json = serde_json::to_string(&req).unwrap();
        assert!(is_shutdown_request(&json).is_some());

        let approved = create_shutdown_approved("sr-1", "worker-1", Some("%5"), None);
        let json = serde_json::to_string(&approved).unwrap();
        assert!(is_shutdown_approved(&json).is_some());

        let rejected = create_shutdown_rejected("sr-1", "worker-1", "still working");
        let json = serde_json::to_string(&rejected).unwrap();
        assert!(is_shutdown_rejected(&json).is_some());
    }

    #[test]
    fn test_format_teammate_messages() {
        let messages = vec![
            TeammateMessage {
                from: "worker-1".to_string(),
                text: "Hello from worker".to_string(),
                timestamp: "2024-01-01T00:00:00Z".to_string(),
                read: false,
                color: Some("red".to_string()),
                summary: Some("Greeting".to_string()),
            },
        ];
        let formatted = format_teammate_messages(&messages);
        assert!(formatted.contains("teammate_id=\"worker-1\""));
        assert!(formatted.contains("color=\"red\""));
        assert!(formatted.contains("summary=\"Greeting\""));
        assert!(formatted.contains("Hello from worker"));
    }

    #[test]
    fn test_mode_set_request_roundtrip() {
        let msg = create_mode_set_request("bypassPermissions", "team-lead");
        let json = serde_json::to_string(&msg).unwrap();
        let parsed = is_mode_set_request(&json);
        assert!(parsed.is_some());
        let parsed = parsed.unwrap();
        assert_eq!(parsed.mode, "bypassPermissions");
        assert_eq!(parsed.from, "team-lead");
    }

    #[test]
    fn test_sandbox_permission_roundtrip() {
        let req = create_sandbox_permission_request(
            "sp-1", "worker-1", "researcher", None, "api.example.com",
        );
        let json = serde_json::to_string(&req).unwrap();
        let parsed = is_sandbox_permission_request(&json);
        assert!(parsed.is_some());
        assert_eq!(parsed.unwrap().host_pattern.host, "api.example.com");

        let resp = create_sandbox_permission_response("sp-1", "api.example.com", true);
        let json = serde_json::to_string(&resp).unwrap();
        let parsed = is_sandbox_permission_response(&json);
        assert!(parsed.is_some());
        assert!(parsed.unwrap().allow);
    }

    #[test]
    fn test_plan_approval_roundtrip() {
        let req = create_plan_approval_request("worker-1", "/tmp/plan.md", "## Plan", "pa-1");
        let json = serde_json::to_string(&req).unwrap();
        assert!(is_plan_approval_request(&json).is_some());

        let resp = create_plan_approval_response("pa-1", true, None, Some("auto"));
        let json = serde_json::to_string(&resp).unwrap();
        assert!(is_plan_approval_response(&json).is_some());
    }

    #[tokio::test]
    async fn test_write_and_read_mailbox() {
        // Use a unique team name to avoid conflicts with parallel tests.
        let team = format!("test-mailbox-{}", uuid::Uuid::new_v4());

        write_to_mailbox(
            "agent-a",
            TeammateMessageInput {
                from: "leader".to_string(),
                text: "Hello agent A".to_string(),
                timestamp: "2024-01-01T00:00:00Z".to_string(),
                color: None,
                summary: None,
            },
            Some(&team),
        )
        .await
        .unwrap();

        let messages = read_mailbox("agent-a", Some(&team)).await;
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].text, "Hello agent A");
        assert!(!messages[0].read);

        // Clean up.
        let _ = tokio::fs::remove_dir_all(get_team_dir(&team)).await;
    }

    #[tokio::test]
    async fn test_mark_as_read() {
        let team = format!("test-mark-read-{}", uuid::Uuid::new_v4());

        write_to_mailbox(
            "agent-b",
            TeammateMessageInput {
                from: "leader".to_string(),
                text: "msg1".to_string(),
                timestamp: "2024-01-01T00:00:00Z".to_string(),
                color: None,
                summary: None,
            },
            Some(&team),
        )
        .await
        .unwrap();

        write_to_mailbox(
            "agent-b",
            TeammateMessageInput {
                from: "leader".to_string(),
                text: "msg2".to_string(),
                timestamp: "2024-01-01T00:00:01Z".to_string(),
                color: None,
                summary: None,
            },
            Some(&team),
        )
        .await
        .unwrap();

        let unread = read_unread_messages("agent-b", Some(&team)).await;
        assert_eq!(unread.len(), 2);

        mark_message_as_read_by_index("agent-b", Some(&team), 0)
            .await
            .unwrap();

        let unread = read_unread_messages("agent-b", Some(&team)).await;
        assert_eq!(unread.len(), 1);
        assert_eq!(unread[0].text, "msg2");

        mark_messages_as_read("agent-b", Some(&team)).await.unwrap();

        let unread = read_unread_messages("agent-b", Some(&team)).await;
        assert!(unread.is_empty());

        // Clean up.
        let _ = tokio::fs::remove_dir_all(get_team_dir(&team)).await;
    }
}
