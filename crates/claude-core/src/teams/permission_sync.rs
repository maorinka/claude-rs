//! Synchronized Permission Prompts for Agent Swarms
//!
//! This module provides infrastructure for coordinating permission prompts
//! across multiple agents in a swarm. When a worker agent needs permission
//! for a tool use, it can forward the request to the team leader, who can
//! then approve or deny it.
//!
//! The system uses a two-layer approach:
//! 1. Filesystem: `pending/` and `resolved/` directories under
//!    `~/.claude/teams/{teamName}/permissions/` for durable state.
//! 2. Mailbox: Messages sent via teammate mailbox for notification.
//!
//! Flow:
//! 1. Worker encounters a permission prompt
//! 2. Worker writes a request to `pending/` and sends a mailbox message to leader
//! 3. Leader polls for pending requests and detects them
//! 4. User approves/denies via the leader's UI
//! 5. Leader writes resolution to `resolved/`, removes from `pending/`, and
//!    sends a mailbox response to the worker
//! 6. Worker polls `resolved/` and continues execution

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use super::mailbox::{
    self, create_permission_request_message, create_permission_response_error,
    create_permission_response_success, create_sandbox_permission_request,
    create_sandbox_permission_response, get_team_dir, read_team_file, TeammateMessageInput,
    TEAM_LEAD_NAME,
};

// ---------------------------------------------------------------------------
// SwarmPermissionRequest
// ---------------------------------------------------------------------------

/// Full request schema for a permission request from a worker to the leader.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwarmPermissionRequest {
    /// Unique identifier for this request.
    pub id: String,
    /// Worker's CLAUDE_CODE_AGENT_ID.
    pub worker_id: String,
    /// Worker's CLAUDE_CODE_AGENT_NAME.
    pub worker_name: String,
    /// Worker's CLAUDE_CODE_AGENT_COLOR.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worker_color: Option<String>,
    /// Team name for routing.
    pub team_name: String,
    /// Tool name requiring permission (e.g. "Bash", "Edit").
    pub tool_name: String,
    /// Original toolUseID from worker's context.
    pub tool_use_id: String,
    /// Human-readable description of the tool use.
    pub description: String,
    /// Serialized tool input.
    pub input: HashMap<String, serde_json::Value>,
    /// Suggested permission rules from the permission result.
    #[serde(default)]
    pub permission_suggestions: Vec<serde_json::Value>,
    /// Status of the request.
    pub status: PermissionStatus,
    /// Who resolved the request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_by: Option<ResolvedBy>,
    /// Timestamp when resolved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<u64>,
    /// Rejection feedback message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feedback: Option<String>,
    /// Modified input if changed by resolver.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_input: Option<HashMap<String, serde_json::Value>>,
    /// "Always allow" rules applied during resolution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_updates: Option<Vec<serde_json::Value>>,
    /// Timestamp when request was created.
    pub created_at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PermissionStatus {
    Pending,
    Approved,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResolvedBy {
    Worker,
    Leader,
}

// ---------------------------------------------------------------------------
// PermissionResolution
// ---------------------------------------------------------------------------

/// Resolution data returned when leader/worker resolves a request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionResolution {
    /// Decision: approved or rejected.
    pub decision: PermissionDecision,
    /// Who resolved it.
    pub resolved_by: ResolvedBy,
    /// Optional feedback message if rejected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feedback: Option<String>,
    /// Optional updated input if the resolver modified it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_input: Option<HashMap<String, serde_json::Value>>,
    /// Permission updates to apply (e.g. "always allow" rules).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_updates: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PermissionDecision {
    Approved,
    Rejected,
}

// ---------------------------------------------------------------------------
// Legacy response type for worker polling
// ---------------------------------------------------------------------------

/// Simpler response format returned by `poll_for_response`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionResponse {
    pub request_id: String,
    pub decision: String, // "approved" or "denied"
    pub timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feedback: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_input: Option<HashMap<String, serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_updates: Option<Vec<serde_json::Value>>,
}

// ---------------------------------------------------------------------------
// Directory helpers
// ---------------------------------------------------------------------------

/// Get the base directory for a team's permission requests:
/// `~/.claude/teams/{teamName}/permissions/`
pub fn get_permission_dir(team_name: &str) -> PathBuf {
    get_team_dir(team_name).join("permissions")
}

/// Get the pending directory for a team.
fn get_pending_dir(team_name: &str) -> PathBuf {
    get_permission_dir(team_name).join("pending")
}

/// Get the resolved directory for a team.
fn get_resolved_dir(team_name: &str) -> PathBuf {
    get_permission_dir(team_name).join("resolved")
}

/// Ensure the permissions directory structure exists.
async fn ensure_permission_dirs(team_name: &str) -> Result<()> {
    for dir in [
        get_permission_dir(team_name),
        get_pending_dir(team_name),
        get_resolved_dir(team_name),
    ] {
        tokio::fs::create_dir_all(&dir).await?;
    }
    Ok(())
}

/// Get the path to a pending request file.
fn get_pending_request_path(team_name: &str, request_id: &str) -> PathBuf {
    get_pending_dir(team_name).join(format!("{}.json", request_id))
}

/// Get the path to a resolved request file.
fn get_resolved_request_path(team_name: &str, request_id: &str) -> PathBuf {
    get_resolved_dir(team_name).join(format!("{}.json", request_id))
}

// ---------------------------------------------------------------------------
// Request ID generation
// ---------------------------------------------------------------------------

/// Generate a unique permission request ID.
pub fn generate_request_id() -> String {
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
    format!("perm-{}-{}", ts, rand_part)
}

/// Generate a unique sandbox permission request ID.
pub fn generate_sandbox_request_id() -> String {
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
    format!("sandbox-{}-{}", ts, rand_part)
}

// ---------------------------------------------------------------------------
// Create permission request
// ---------------------------------------------------------------------------

/// Create a new `SwarmPermissionRequest` object.
#[allow(clippy::too_many_arguments)]
pub fn create_permission_request(
    tool_name: &str,
    tool_use_id: &str,
    input: HashMap<String, serde_json::Value>,
    description: &str,
    permission_suggestions: Vec<serde_json::Value>,
    team_name: &str,
    worker_id: &str,
    worker_name: &str,
    worker_color: Option<&str>,
) -> SwarmPermissionRequest {
    SwarmPermissionRequest {
        id: generate_request_id(),
        worker_id: worker_id.to_string(),
        worker_name: worker_name.to_string(),
        worker_color: worker_color.map(|s| s.to_string()),
        team_name: team_name.to_string(),
        tool_name: tool_name.to_string(),
        tool_use_id: tool_use_id.to_string(),
        description: description.to_string(),
        input,
        permission_suggestions,
        status: PermissionStatus::Pending,
        resolved_by: None,
        resolved_at: None,
        feedback: None,
        updated_input: None,
        permission_updates: None,
        created_at: chrono::Utc::now().timestamp_millis() as u64,
    }
}

// ---------------------------------------------------------------------------
// Write / read permission requests (filesystem layer)
// ---------------------------------------------------------------------------

/// Write a permission request to the pending directory with file locking.
/// Called by worker agents when they need permission approval from the leader.
pub async fn write_permission_request(request: &SwarmPermissionRequest) -> Result<()> {
    ensure_permission_dirs(&request.team_name).await?;

    let pending_path = get_pending_request_path(&request.team_name, &request.id);
    let lock_path = get_pending_dir(&request.team_name).join(".lock");

    // Create a lock file for the pending directory.
    if !lock_path.exists() {
        let _ = tokio::fs::write(&lock_path, "").await;
    }

    // Simple lock-via-rename approach: write to a temp file, then rename.
    let temp_path = pending_path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(request)?;
    tokio::fs::write(&temp_path, &json).await?;
    tokio::fs::rename(&temp_path, &pending_path).await?;

    debug!(
        "[PermissionSync] Wrote pending request {} from {} for {}",
        request.id, request.worker_name, request.tool_name
    );
    Ok(())
}

/// Read all pending permission requests for a team.
/// Called by the team leader to see what requests need attention.
pub async fn read_pending_permissions(team_name: &str) -> Vec<SwarmPermissionRequest> {
    let pending_dir = get_pending_dir(team_name);

    let read_result: std::io::Result<tokio::fs::ReadDir> = tokio::fs::read_dir(&pending_dir).await;

    let mut entries = match read_result {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            warn!("[PermissionSync] Failed to read pending requests: {}", e);
            return Vec::new();
        },
    };

    let mut requests = Vec::new();

    loop {
        let entry = match entries.next_entry().await {
            Ok(Some(e)) => e,
            Ok(None) => break,
            Err(_) => break,
        };

        let path = entry.path();
        let file_name = path
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();

        if !file_name.ends_with(".json") || file_name == ".lock" {
            continue;
        }

        let content = match tokio::fs::read_to_string(&path).await {
            Ok(c) => c,
            Err(e) => {
                warn!(
                    "[PermissionSync] Failed to read request file {}: {}",
                    file_name, e
                );
                continue;
            },
        };

        match serde_json::from_str::<SwarmPermissionRequest>(&content) {
            Ok(req) => requests.push(req),
            Err(e) => {
                warn!("[PermissionSync] Invalid request file {}: {}", file_name, e);
            },
        }
    }

    // Sort by creation time (oldest first).
    requests.sort_by_key(|r| r.created_at);
    requests
}

/// Read a resolved permission request by ID.
/// Called by workers to check if their request has been resolved.
pub async fn read_resolved_permission(
    request_id: &str,
    team_name: &str,
) -> Option<SwarmPermissionRequest> {
    let resolved_path = get_resolved_request_path(team_name, request_id);

    match tokio::fs::read_to_string(&resolved_path).await {
        Ok(content) => match serde_json::from_str::<SwarmPermissionRequest>(&content) {
            Ok(req) => Some(req),
            Err(e) => {
                warn!(
                    "[PermissionSync] Invalid resolved request {}: {}",
                    request_id, e
                );
                None
            },
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            warn!(
                "[PermissionSync] Failed to read resolved request {}: {}",
                request_id, e
            );
            None
        },
    }
}

// ---------------------------------------------------------------------------
// Resolve a permission request
// ---------------------------------------------------------------------------

/// Resolve a permission request. Writes the resolution to `resolved/`,
/// removes from `pending/`. Called by the team leader (or worker in
/// self-resolution cases).
pub async fn resolve_permission(
    request_id: &str,
    resolution: &PermissionResolution,
    team_name: &str,
) -> bool {
    if let Err(e) = ensure_permission_dirs(team_name).await {
        warn!("[PermissionSync] Failed to ensure dirs: {}", e);
        return false;
    }

    let pending_path = get_pending_request_path(team_name, request_id);
    let resolved_path = get_resolved_request_path(team_name, request_id);

    // Read the pending request.
    let content = match tokio::fs::read_to_string(&pending_path).await {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            debug!("[PermissionSync] Pending request not found: {}", request_id);
            return false;
        },
        Err(e) => {
            warn!(
                "[PermissionSync] Failed to read pending {}: {}",
                request_id, e
            );
            return false;
        },
    };

    let mut request: SwarmPermissionRequest = match serde_json::from_str(&content) {
        Ok(r) => r,
        Err(e) => {
            warn!(
                "[PermissionSync] Invalid pending request {}: {}",
                request_id, e
            );
            return false;
        },
    };

    // Update the request with resolution data.
    request.status = match resolution.decision {
        PermissionDecision::Approved => PermissionStatus::Approved,
        PermissionDecision::Rejected => PermissionStatus::Rejected,
    };
    request.resolved_by = Some(resolution.resolved_by);
    request.resolved_at = Some(chrono::Utc::now().timestamp_millis() as u64);
    request.feedback = resolution.feedback.clone();
    request.updated_input = resolution.updated_input.clone();
    request.permission_updates = resolution.permission_updates.clone();

    // Write to resolved directory (atomic via temp + rename).
    let temp_path = resolved_path.with_extension("json.tmp");
    let json = match serde_json::to_string_pretty(&request) {
        Ok(j) => j,
        Err(e) => {
            warn!("[PermissionSync] Serialize failed: {}", e);
            return false;
        },
    };

    if let Err(e) = tokio::fs::write(&temp_path, &json).await {
        warn!("[PermissionSync] Failed to write resolved temp: {}", e);
        return false;
    }
    if let Err(e) = tokio::fs::rename(&temp_path, &resolved_path).await {
        warn!("[PermissionSync] Failed to rename to resolved: {}", e);
        return false;
    }

    // Remove from pending directory.
    if let Err(e) = tokio::fs::remove_file(&pending_path).await {
        warn!(
            "[PermissionSync] Failed to remove pending {}: {}",
            request_id, e
        );
        // Not fatal -- the resolved file is already in place.
    }

    debug!(
        "[PermissionSync] Resolved request {} with {:?}",
        request_id, resolution.decision
    );
    true
}

// ---------------------------------------------------------------------------
// Poll for response (worker side)
// ---------------------------------------------------------------------------

/// Poll for a permission response (worker-side convenience function).
/// Converts the resolved request into a simpler response format.
pub async fn poll_for_response(request_id: &str, team_name: &str) -> Option<PermissionResponse> {
    let resolved = read_resolved_permission(request_id, team_name).await?;

    let decision = if resolved.status == PermissionStatus::Approved {
        "approved"
    } else {
        "denied"
    };

    let timestamp = if let Some(at) = resolved.resolved_at {
        chrono::DateTime::from_timestamp_millis(at as i64)
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339())
    } else {
        chrono::DateTime::from_timestamp_millis(resolved.created_at as i64)
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339())
    };

    Some(PermissionResponse {
        request_id: resolved.id,
        decision: decision.to_string(),
        timestamp,
        feedback: resolved.feedback,
        updated_input: resolved.updated_input,
        permission_updates: resolved.permission_updates,
    })
}

/// Remove a worker's response after processing (delete the resolved file).
pub async fn remove_worker_response(request_id: &str, team_name: &str) -> bool {
    delete_resolved_permission(request_id, team_name).await
}

// ---------------------------------------------------------------------------
// Delete resolved permission
// ---------------------------------------------------------------------------

/// Delete a resolved permission file.
/// Called after a worker has processed the resolution.
pub async fn delete_resolved_permission(request_id: &str, team_name: &str) -> bool {
    let resolved_path = get_resolved_request_path(team_name, request_id);
    match tokio::fs::remove_file(&resolved_path).await {
        Ok(_) => {
            debug!(
                "[PermissionSync] Deleted resolved permission: {}",
                request_id
            );
            true
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => {
            warn!(
                "[PermissionSync] Failed to delete resolved permission: {}",
                e
            );
            false
        },
    }
}

// ---------------------------------------------------------------------------
// Cleanup old resolutions
// ---------------------------------------------------------------------------

/// Clean up old resolved permission files.
/// Called periodically to prevent file accumulation.
///
/// `max_age_ms` defaults to 1 hour (3_600_000 ms).
pub async fn cleanup_old_resolutions(team_name: &str, max_age_ms: u64) -> usize {
    let resolved_dir = get_resolved_dir(team_name);

    let read_result: std::io::Result<tokio::fs::ReadDir> = tokio::fs::read_dir(&resolved_dir).await;

    let mut entries = match read_result {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return 0,
        Err(e) => {
            warn!("[PermissionSync] Failed to cleanup resolutions: {}", e);
            return 0;
        },
    };

    let now = chrono::Utc::now().timestamp_millis() as u64;
    let mut cleaned = 0usize;

    loop {
        let entry = match entries.next_entry().await {
            Ok(Some(e)) => e,
            Ok(None) => break,
            Err(_) => break,
        };

        let path = entry.path();
        let file_name = path
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();

        if !file_name.ends_with(".json") {
            continue;
        }

        let should_clean = match tokio::fs::read_to_string(&path).await {
            Ok(content) => match serde_json::from_str::<SwarmPermissionRequest>(&content) {
                Ok(request) => {
                    let resolved_at = request.resolved_at.unwrap_or(request.created_at);
                    now.saturating_sub(resolved_at) >= max_age_ms
                },
                Err(_) => true,
            },
            Err(_) => true,
        };

        if should_clean && tokio::fs::remove_file(&path).await.is_ok() {
            debug!("[PermissionSync] Cleaned up old resolution: {}", file_name);
            cleaned += 1;
        }
    }

    if cleaned > 0 {
        debug!("[PermissionSync] Cleaned up {} old resolutions", cleaned);
    }
    cleaned
}

// ---------------------------------------------------------------------------
// Team leader / worker detection
// ---------------------------------------------------------------------------

/// Check if a given agent_id represents a team leader.
pub fn is_team_leader(agent_id: Option<&str>) -> bool {
    match agent_id {
        None => true,
        Some(id) => id.is_empty() || id == "team-lead",
    }
}

/// Check if the given agent is a worker in a swarm.
pub fn is_swarm_worker(team_name: Option<&str>, agent_id: Option<&str>) -> bool {
    team_name.is_some() && agent_id.is_some() && !is_team_leader(agent_id)
}

// ---------------------------------------------------------------------------
// Mailbox-based permission system
// ---------------------------------------------------------------------------

/// Get the leader's name from the team file.
pub async fn get_leader_name(team_name: &str) -> Option<String> {
    let team_file = read_team_file(team_name).await?;
    let lead = team_file
        .members
        .iter()
        .find(|m| m.agent_id == team_file.lead_agent_id);
    Some(lead.map_or_else(|| "team-lead".to_string(), |m| m.name.clone()))
}

/// Send a permission request to the leader via mailbox.
pub async fn send_permission_request_via_mailbox(request: &SwarmPermissionRequest) -> bool {
    let leader_name = match get_leader_name(&request.team_name).await {
        Some(n) => n,
        None => {
            debug!("[PermissionSync] Cannot send permission request: leader name not found");
            return false;
        },
    };

    let message = create_permission_request_message(
        &request.id,
        &request.worker_name,
        &request.tool_name,
        &request.tool_use_id,
        &request.description,
        request.input.clone(),
        request.permission_suggestions.clone(),
    );

    let msg_json = match serde_json::to_string(&message) {
        Ok(j) => j,
        Err(e) => {
            warn!(
                "[PermissionSync] Failed to serialize permission request: {}",
                e
            );
            return false;
        },
    };

    match mailbox::write_to_mailbox(
        &leader_name,
        TeammateMessageInput {
            from: request.worker_name.clone(),
            text: msg_json,
            timestamp: chrono::Utc::now().to_rfc3339(),
            color: request.worker_color.clone(),
            summary: None,
        },
        Some(&request.team_name),
    )
    .await
    {
        Ok(_) => {
            debug!(
                "[PermissionSync] Sent permission request {} to leader {} via mailbox",
                request.id, leader_name
            );
            true
        },
        Err(e) => {
            warn!(
                "[PermissionSync] Failed to send permission request via mailbox: {}",
                e
            );
            false
        },
    }
}

/// Send a permission response to a worker via mailbox.
pub async fn send_permission_response_via_mailbox(
    worker_name: &str,
    resolution: &PermissionResolution,
    request_id: &str,
    team_name: &str,
    sender_name: Option<&str>,
) -> bool {
    let msg = match resolution.decision {
        PermissionDecision::Approved => {
            let resp = create_permission_response_success(
                request_id,
                resolution.updated_input.clone(),
                resolution.permission_updates.clone(),
            );
            match serde_json::to_string(&resp) {
                Ok(j) => j,
                Err(e) => {
                    warn!("[PermissionSync] Serialize error: {}", e);
                    return false;
                },
            }
        },
        PermissionDecision::Rejected => {
            let error = resolution
                .feedback
                .as_deref()
                .unwrap_or("Permission denied");
            let resp = create_permission_response_error(request_id, error);
            match serde_json::to_string(&resp) {
                Ok(j) => j,
                Err(e) => {
                    warn!("[PermissionSync] Serialize error: {}", e);
                    return false;
                },
            }
        },
    };

    let sender = sender_name.unwrap_or(TEAM_LEAD_NAME);

    match mailbox::write_to_mailbox(
        worker_name,
        TeammateMessageInput {
            from: sender.to_string(),
            text: msg,
            timestamp: chrono::Utc::now().to_rfc3339(),
            color: None,
            summary: None,
        },
        Some(team_name),
    )
    .await
    {
        Ok(_) => {
            debug!(
                "[PermissionSync] Sent permission response for {} to worker {} via mailbox",
                request_id, worker_name
            );
            true
        },
        Err(e) => {
            warn!(
                "[PermissionSync] Failed to send permission response via mailbox: {}",
                e
            );
            false
        },
    }
}

// ---------------------------------------------------------------------------
// Sandbox permission mailbox system
// ---------------------------------------------------------------------------

/// Send a sandbox permission request to the leader via mailbox.
pub async fn send_sandbox_permission_request_via_mailbox(
    host: &str,
    request_id: &str,
    team_name: &str,
    worker_id: &str,
    worker_name: &str,
    worker_color: Option<&str>,
) -> bool {
    let leader_name = match get_leader_name(team_name).await {
        Some(n) => n,
        None => {
            debug!(
                "[PermissionSync] Cannot send sandbox permission request: leader name not found"
            );
            return false;
        },
    };

    let message =
        create_sandbox_permission_request(request_id, worker_id, worker_name, worker_color, host);

    let msg_json = match serde_json::to_string(&message) {
        Ok(j) => j,
        Err(e) => {
            warn!(
                "[PermissionSync] Failed to serialize sandbox request: {}",
                e
            );
            return false;
        },
    };

    match mailbox::write_to_mailbox(
        &leader_name,
        TeammateMessageInput {
            from: worker_name.to_string(),
            text: msg_json,
            timestamp: chrono::Utc::now().to_rfc3339(),
            color: worker_color.map(|s| s.to_string()),
            summary: None,
        },
        Some(team_name),
    )
    .await
    {
        Ok(_) => {
            debug!(
                "[PermissionSync] Sent sandbox permission request {} for host {} to leader {}",
                request_id, host, leader_name
            );
            true
        },
        Err(e) => {
            warn!(
                "[PermissionSync] Failed to send sandbox permission request: {}",
                e
            );
            false
        },
    }
}

/// Send a sandbox permission response to a worker via mailbox.
pub async fn send_sandbox_permission_response_via_mailbox(
    worker_name: &str,
    request_id: &str,
    host: &str,
    allow: bool,
    team_name: &str,
    sender_name: Option<&str>,
) -> bool {
    let message = create_sandbox_permission_response(request_id, host, allow);

    let msg_json = match serde_json::to_string(&message) {
        Ok(j) => j,
        Err(e) => {
            warn!("[PermissionSync] Serialize error: {}", e);
            return false;
        },
    };

    let sender = sender_name.unwrap_or(TEAM_LEAD_NAME);

    match mailbox::write_to_mailbox(
        worker_name,
        TeammateMessageInput {
            from: sender.to_string(),
            text: msg_json,
            timestamp: chrono::Utc::now().to_rfc3339(),
            color: None,
            summary: None,
        },
        Some(team_name),
    )
    .await
    {
        Ok(_) => {
            debug!(
                "[PermissionSync] Sent sandbox response for {} (host: {}, allow: {}) to {}",
                request_id, host, allow, worker_name
            );
            true
        },
        Err(e) => {
            warn!("[PermissionSync] Failed to send sandbox response: {}", e);
            false
        },
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_id_generation() {
        let id = generate_request_id();
        assert!(id.starts_with("perm-"));
        // Should have timestamp + random
        assert!(id.len() > 10);
    }

    #[test]
    fn test_sandbox_request_id_generation() {
        let id = generate_sandbox_request_id();
        assert!(id.starts_with("sandbox-"));
    }

    #[test]
    fn test_is_team_leader() {
        assert!(is_team_leader(None));
        assert!(is_team_leader(Some("")));
        assert!(is_team_leader(Some("team-lead")));
        assert!(!is_team_leader(Some("worker-1@my-team")));
    }

    #[test]
    fn test_is_swarm_worker() {
        assert!(is_swarm_worker(Some("team"), Some("worker-1")));
        assert!(!is_swarm_worker(None, Some("worker-1")));
        assert!(!is_swarm_worker(Some("team"), None));
        assert!(!is_swarm_worker(Some("team"), Some("team-lead")));
    }

    #[test]
    fn test_create_permission_request() {
        let req = create_permission_request(
            "Bash",
            "tu-1",
            HashMap::new(),
            "run npm test",
            vec![],
            "my-team",
            "worker-1@my-team",
            "worker-1",
            Some("red"),
        );
        assert!(req.id.starts_with("perm-"));
        assert_eq!(req.tool_name, "Bash");
        assert_eq!(req.status, PermissionStatus::Pending);
        assert_eq!(req.worker_color.as_deref(), Some("red"));
    }

    #[tokio::test]
    async fn test_write_and_read_permission() {
        let team = format!("test-perm-{}", uuid::Uuid::new_v4());

        let req = create_permission_request(
            "Bash",
            "tu-1",
            HashMap::new(),
            "npm test",
            vec![],
            &team,
            "w1@team",
            "w1",
            None,
        );

        write_permission_request(&req).await.unwrap();

        let pending = read_pending_permissions(&team).await;
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, req.id);

        // Resolve it.
        let resolved = resolve_permission(
            &req.id,
            &PermissionResolution {
                decision: PermissionDecision::Approved,
                resolved_by: ResolvedBy::Leader,
                feedback: None,
                updated_input: None,
                permission_updates: None,
            },
            &team,
        )
        .await;
        assert!(resolved);

        // Pending should be empty now.
        let pending = read_pending_permissions(&team).await;
        assert!(pending.is_empty());

        // Should be readable from resolved.
        let resp = poll_for_response(&req.id, &team).await;
        assert!(resp.is_some());
        assert_eq!(resp.unwrap().decision, "approved");

        // Clean up.
        let _ = tokio::fs::remove_dir_all(get_team_dir(&team)).await;
    }

    #[tokio::test]
    async fn test_cleanup_old_resolutions() {
        let team = format!("test-cleanup-{}", uuid::Uuid::new_v4());

        let req = create_permission_request(
            "Bash",
            "tu-1",
            HashMap::new(),
            "npm test",
            vec![],
            &team,
            "w1@team",
            "w1",
            None,
        );

        write_permission_request(&req).await.unwrap();

        resolve_permission(
            &req.id,
            &PermissionResolution {
                decision: PermissionDecision::Approved,
                resolved_by: ResolvedBy::Leader,
                feedback: None,
                updated_input: None,
                permission_updates: None,
            },
            &team,
        )
        .await;

        // Clean up with max_age = 0 (clean everything).
        let count = cleanup_old_resolutions(&team, 0).await;
        assert_eq!(count, 1);

        // Clean up the test dirs.
        let _ = tokio::fs::remove_dir_all(get_team_dir(&team)).await;
    }
}
