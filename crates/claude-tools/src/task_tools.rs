use anyhow::Result;
use async_trait::async_trait;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::hooks::{get_global_runner, types::HookEvent};
use claude_core::types::events::ToolResultData;

// Verbatim ports of TS tools/Task{Create,Get,List,Update}Tool/prompt.ts.
// TS TaskCreate + TaskList branch on `isAgentSwarmsEnabled()`; the
// embedded Rust port has teams always enabled, so the "teammate"
// paragraphs are baked in.
pub const TASK_CREATE_PROMPT: &str = include_str!("prompts/task_create.md");
pub const TASK_GET_PROMPT: &str = include_str!("prompts/task_get.md");
pub const TASK_LIST_PROMPT: &str = include_str!("prompts/task_list.md");
pub const TASK_UPDATE_PROMPT: &str = include_str!("prompts/task_update.md");

// ─── Shared state ─────────────────────────────────────────────────────────────

/// A task entry with optional real process tracking.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskEntry {
    pub id: String,
    pub subject: String,
    pub description: String,
    #[serde(
        rename = "activeForm",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub active_form: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    pub status: String, // "pending", "in_progress", "completed", "stopped"
    #[serde(default)]
    pub blocks: Vec<String>,
    #[serde(rename = "blockedBy", default)]
    pub blocked_by: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    /// Captured stdout/stderr output from the background process (if any).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// Process ID for background tasks spawned by the system.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
}

impl TaskEntry {
    fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "subject": self.subject,
            "description": self.description,
            "activeForm": self.active_form,
            "owner": self.owner,
            "status": self.status,
            "blocks": self.blocks,
            "blockedBy": self.blocked_by,
            "metadata": self.metadata,
            "createdAt": self.created_at,
            "output": self.output,
            "pid": self.pid,
        })
    }
}

fn new_task_entry(id: String, subject: String, description: String) -> TaskEntry {
    TaskEntry {
        id,
        subject,
        description,
        active_form: None,
        owner: None,
        status: "pending".to_string(),
        blocks: Vec::new(),
        blocked_by: Vec::new(),
        metadata: None,
        created_at: now_iso(),
        output: None,
        pid: None,
    }
}

static TASK_STORE: Lazy<Mutex<HashMap<String, TaskEntry>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
static NEXT_TASK_ID: AtomicU64 = AtomicU64::new(1);
const HIGH_WATER_MARK_FILE: &str = ".highwatermark";

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn new_task_id() -> String {
    let disk_max = list_task_entries()
        .into_iter()
        .filter_map(|task| task.id.parse::<u64>().ok())
        .max()
        .unwrap_or(0);
    let minimum = disk_max.max(read_high_water_mark()) + 1;
    let candidate = NEXT_TASK_ID
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
            let next = current.max(minimum);
            Some(next + 1)
        })
        .unwrap_or(minimum)
        .max(minimum);
    write_high_water_mark(candidate);
    candidate.to_string()
}

fn error_result(msg: impl Into<String>) -> ToolResultData {
    ToolResultData {
        data: json!({ "error": msg.into() }),
        is_error: true,
    }
}

fn task_list_id() -> String {
    std::env::var("CLAUDE_CODE_TASK_LIST_ID")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| {
            std::env::var("CLAUDE_CODE_TEAM_NAME")
                .ok()
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_else(|| claude_core::api::client::get_session_id().clone())
}

fn sanitize_path_component(input: &str) -> String {
    input
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

fn tasks_dir() -> PathBuf {
    let root = {
        #[cfg(test)]
        {
            std::env::var("CLAUDE_RS_TEST_TASKS_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| std::env::temp_dir().join("claude-rs-test-tasks"))
        }
        #[cfg(not(test))]
        {
            claude_core::errors_util::get_claude_config_home_dir().join("tasks")
        }
    };
    root.join(sanitize_path_component(&task_list_id()))
}

fn task_path(task_id: &str) -> PathBuf {
    tasks_dir().join(format!("{}.json", sanitize_path_component(task_id)))
}

fn high_water_mark_path() -> PathBuf {
    tasks_dir().join(HIGH_WATER_MARK_FILE)
}

fn read_high_water_mark() -> u64 {
    std::fs::read_to_string(high_water_mark_path())
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(0)
}

fn write_high_water_mark(value: u64) {
    let dir = tasks_dir();
    if std::fs::create_dir_all(&dir).is_ok() {
        let _ = std::fs::write(high_water_mark_path(), value.to_string());
    }
}

fn read_task_file(path: &Path) -> Option<TaskEntry> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn load_task_entry(task_id: &str) -> Option<TaskEntry> {
    if let Some(entry) = TASK_STORE.lock().ok()?.get(task_id).cloned() {
        return Some(entry);
    }
    read_task_file(&task_path(task_id))
}

fn list_task_entries() -> Vec<TaskEntry> {
    let mut tasks = HashMap::new();
    let dir = tasks_dir();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            if let Some(task) = read_task_file(&path) {
                tasks.insert(task.id.clone(), task);
            }
        }
    }
    if let Ok(store) = TASK_STORE.lock() {
        for task in store.values() {
            tasks.insert(task.id.clone(), task.clone());
        }
    }
    tasks.into_values().collect()
}

fn save_task_entry(entry: &TaskEntry) {
    let dir = tasks_dir();
    if std::fs::create_dir_all(&dir).is_ok() {
        if let Ok(text) = serde_json::to_string_pretty(entry) {
            let _ = std::fs::write(task_path(&entry.id), text);
        }
    }
    if let Ok(mut store) = TASK_STORE.lock() {
        store.insert(entry.id.clone(), entry.clone());
    }
}

fn delete_task_entry(task_id: &str) -> bool {
    if let Ok(numeric_id) = task_id.parse::<u64>() {
        let current = read_high_water_mark();
        if numeric_id > current {
            write_high_water_mark(numeric_id);
        }
    }
    let file_deleted = match std::fs::remove_file(task_path(task_id)) {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => false,
    };
    let store_deleted = TASK_STORE
        .lock()
        .ok()
        .and_then(|mut store| store.remove(task_id))
        .is_some();
    let deleted = file_deleted || store_deleted;
    if !deleted {
        return false;
    }
    for mut task in list_task_entries() {
        let original_blocks = task.blocks.len();
        let original_blocked_by = task.blocked_by.len();
        task.blocks.retain(|id| id != task_id);
        task.blocked_by.retain(|id| id != task_id);
        if task.blocks.len() != original_blocks || task.blocked_by.len() != original_blocked_by {
            save_task_entry(&task);
        }
    }
    true
}

fn block_task(blocker_id: &str, blocked_id: &str) -> bool {
    let Some(mut blocker) = load_task_entry(blocker_id) else {
        return false;
    };
    let Some(mut blocked) = load_task_entry(blocked_id) else {
        return false;
    };
    let mut changed = false;
    if !blocker.blocks.iter().any(|id| id == blocked_id) {
        blocker.blocks.push(blocked_id.to_string());
        changed = true;
    }
    if !blocked.blocked_by.iter().any(|id| id == blocker_id) {
        blocked.blocked_by.push(blocker_id.to_string());
        changed = true;
    }
    save_task_entry(&blocker);
    save_task_entry(&blocked);
    changed
}

fn value_is_js_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64() != Some(0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

// ─── Public helpers for process-tracking integration ──────────────────────────

/// Update a task's output and PID.  Called by the agent/background-spawn
/// infrastructure after it launches a child process.
pub fn register_process(task_id: &str, pid: u32) {
    if let Some(mut entry) = load_task_entry(task_id) {
        entry.pid = Some(pid);
        entry.status = "in_progress".to_string();
        save_task_entry(&entry);
    }
}

/// Create a new task entry in the store and return its ID.
/// Used by background agent spawning to register tasks that can be
/// queried via TaskGet / TaskStop / TaskOutput.
pub fn create_task_entry(subject: &str, description: &str) -> String {
    let id = new_task_id();
    let entry = new_task_entry(id.clone(), subject.to_string(), description.to_string());
    save_task_entry(&entry);
    id
}

/// Look up a task entry by ID.
pub fn get_task_entry(task_id: &str) -> Option<TaskEntry> {
    load_task_entry(task_id)
}

/// Append captured output to a task entry.
pub fn append_output(task_id: &str, text: &str) {
    if let Some(mut entry) = load_task_entry(task_id) {
        let buf = entry.output.get_or_insert_with(String::new);
        buf.push_str(text);
        save_task_entry(&entry);
    }
}

pub fn update_task_status(task_id: &str, status: &str) {
    if let Some(mut entry) = load_task_entry(task_id) {
        entry.status = status.to_string();
        save_task_entry(&entry);
    }
}

// ─── TaskCreateTool ────────────────────────────────────────────────────────────

pub struct TaskCreateTool;

#[async_trait]
impl ToolExecutor for TaskCreateTool {
    fn name(&self) -> &str {
        "TaskCreate"
    }

    fn description(&self) -> String {
        TASK_CREATE_PROMPT.to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "subject": {
                    "type": "string",
                    "description": "Short title for the task"
                },
                "description": {
                    "type": "string",
                    "description": "Detailed description of the task"
                },
                "activeForm": {
                    "type": "string",
                    "description": "Present continuous form for spinner"
                },
                "metadata": {
                    "type": "object",
                    "description": "Arbitrary metadata to attach to the task"
                }
            },
            "required": ["subject", "description"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }
    fn is_read_only(&self, _input: &Value) -> bool {
        false
    }

    fn to_auto_classifier_input(&self, input: &Value) -> Option<String> {
        Some(input["subject"].as_str().unwrap_or_default().to_string())
    }

    async fn call(
        &self,
        input: &Value,
        _ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let subject = match input["subject"].as_str() {
            Some(s) => s.to_string(),
            None => return Ok(error_result("missing required parameter: subject")),
        };
        let description = match input["description"].as_str() {
            Some(d) => d.to_string(),
            None => return Ok(error_result("missing required parameter: description")),
        };

        let id = new_task_id();
        let mut entry = new_task_entry(id.clone(), subject.clone(), description.clone());
        entry.active_form = input["activeForm"].as_str().map(str::to_string);
        entry.metadata = input
            .get("metadata")
            .filter(|value| !value.is_null())
            .cloned();

        save_task_entry(&entry);

        // Fire TaskCreated hooks if a runner has been installed. Ports TS
        // TaskCreateTool.ts lines 93-113 (executeTaskCreatedHooks). If a hook
        // returns a blocking error we delete the task and surface the message,
        // matching the TS `deleteTask(...); throw new Error(blockingErrors)`
        // path.
        if let Some(runner) = get_global_runner() {
            let extra = json!({
                "task_id": id,
                "task_subject": subject,
                "task_description": description,
            });
            let aggregated = runner
                .run_hooks(&HookEvent::TaskCreated, extra, None, None, None, None)
                .await;
            if !aggregated.blocking_errors.is_empty() {
                // Roll the task back so failed hooks don't leave a dangling entry.
                delete_task_entry(&id);
                let msg = aggregated
                    .blocking_errors
                    .iter()
                    .map(claude_core::hooks::get_task_created_hook_message)
                    .collect::<Vec<_>>()
                    .join("\n");
                return Ok(error_result(msg));
            }
        }

        Ok(ToolResultData {
            data: json!({
                "task": {
                    "id": id,
                    "subject": subject,
                }
            }),
            is_error: false,
        })
    }
}

// ─── TaskListTool ──────────────────────────────────────────────────────────────

pub struct TaskListTool;

#[async_trait]
impl ToolExecutor for TaskListTool {
    fn name(&self) -> &str {
        "TaskList"
    }

    fn description(&self) -> String {
        TASK_LIST_PROMPT.to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {}
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }
    fn is_read_only(&self, _input: &Value) -> bool {
        true
    }

    async fn call(
        &self,
        _input: &Value,
        _ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let mut entries = list_task_entries();
        let completed = entries
            .iter()
            .filter(|task| task.status == "completed")
            .map(|task| task.id.clone())
            .collect::<std::collections::HashSet<_>>();
        entries.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        let tasks = entries
            .into_iter()
            .filter(|task| {
                !task
                    .metadata
                    .as_ref()
                    .and_then(Value::as_object)
                    .and_then(|metadata| metadata.get("_internal"))
                    .is_some_and(value_is_js_truthy)
            })
            .map(|task| {
                let blocked_by = task
                    .blocked_by
                    .into_iter()
                    .filter(|id| !completed.contains(id))
                    .collect::<Vec<_>>();
                json!({
                    "id": task.id,
                    "subject": task.subject,
                    "status": task.status,
                    "owner": task.owner,
                    "blockedBy": blocked_by,
                })
            })
            .collect::<Vec<_>>();

        Ok(ToolResultData {
            data: json!({ "tasks": tasks }),
            is_error: false,
        })
    }
}

// ─── TaskUpdateTool ────────────────────────────────────────────────────────────

pub struct TaskUpdateTool;

#[async_trait]
impl ToolExecutor for TaskUpdateTool {
    fn name(&self) -> &str {
        "TaskUpdate"
    }

    fn description(&self) -> String {
        TASK_UPDATE_PROMPT.to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "taskId": {
                    "type": "string",
                    "description": "ID of the task to update"
                },
                "status": {
                    "type": "string",
                    "description": "New status: pending, in_progress, completed, stopped"
                },
                "subject": {
                    "type": "string",
                    "description": "Updated subject"
                },
                "description": {
                    "type": "string",
                    "description": "Updated description"
                },
                "activeForm": {
                    "type": "string",
                    "description": "Present continuous form for spinner"
                },
                "owner": {
                    "type": "string",
                    "description": "Agent or teammate owner"
                },
                "addBlocks": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Task IDs this task blocks"
                },
                "addBlockedBy": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Task IDs that block this task"
                },
                "metadata": {
                    "type": "object",
                    "description": "Metadata fields to merge; null values delete keys"
                }
            },
            "required": ["taskId"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }
    fn is_read_only(&self, _input: &Value) -> bool {
        false
    }

    fn to_auto_classifier_input(&self, input: &Value) -> Option<String> {
        let mut parts = Vec::new();
        if let Some(task_id) = input["taskId"].as_str() {
            parts.push(task_id);
        }
        if let Some(status) = input["status"].as_str() {
            parts.push(status);
        }
        if let Some(subject) = input["subject"].as_str() {
            parts.push(subject);
        }
        Some(parts.join(" "))
    }

    async fn call(
        &self,
        input: &Value,
        _ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let task_id = match input["taskId"].as_str() {
            Some(id) => id.to_string(),
            None => return Ok(error_result("missing required parameter: taskId")),
        };

        let requested_status = input["status"].as_str().map(str::to_string);
        let task_before_update = match load_task_entry(&task_id) {
            Some(entry) => entry,
            None => {
                return Ok(ToolResultData {
                    data: json!({
                        "success": false,
                        "taskId": task_id,
                        "updatedFields": [],
                        "error": "Task not found",
                    }),
                    is_error: false,
                });
            }
        };

        if requested_status.as_deref() == Some("completed")
            && task_before_update.status != "completed"
        {
            if let Some(runner) = get_global_runner() {
                let extra = json!({
                    "task_id": task_id,
                    "task_subject": task_before_update.subject,
                    "task_description": task_before_update.description,
                });
                let aggregated = runner
                    .run_hooks(&HookEvent::TaskCompleted, extra, None, None, None, None)
                    .await;
                if !aggregated.blocking_errors.is_empty() {
                    let msg = aggregated
                        .blocking_errors
                        .iter()
                        .map(claude_core::hooks::get_task_completed_hook_message)
                        .collect::<Vec<_>>()
                        .join("\n");
                    return Ok(ToolResultData {
                        data: json!({
                            "success": false,
                            "taskId": task_id,
                            "updatedFields": [],
                            "error": msg,
                        }),
                        is_error: false,
                    });
                }
            }
        }

        let mut entry = match load_task_entry(&task_id) {
            Some(e) => e,
            None => {
                return Ok(ToolResultData {
                    data: json!({
                        "success": false,
                        "taskId": task_id,
                        "updatedFields": [],
                        "error": "Task not found",
                    }),
                    is_error: false,
                });
            }
        };

        let old_status = entry.status.clone();
        let mut updated_fields = Vec::new();
        if let Some(status) = requested_status {
            if status == "deleted" {
                let deleted = delete_task_entry(&task_id);
                let mut data = json!({
                    "success": deleted,
                    "taskId": task_id,
                    "updatedFields": if deleted { json!(["deleted"]) } else { json!([]) },
                });
                if deleted {
                    data["statusChange"] = json!({
                        "from": old_status,
                        "to": "deleted",
                    });
                } else {
                    data["error"] = json!("Failed to delete task");
                }
                return Ok(ToolResultData {
                    data,
                    is_error: false,
                });
            }
            if entry.status != status {
                entry.status = status;
                updated_fields.push("status");
            }
        }
        if let Some(subject) = input["subject"].as_str() {
            if entry.subject != subject {
                entry.subject = subject.to_string();
                updated_fields.push("subject");
            }
        }
        if let Some(description) = input["description"].as_str() {
            if entry.description != description {
                entry.description = description.to_string();
                updated_fields.push("description");
            }
        }
        if let Some(active_form) = input["activeForm"].as_str() {
            if entry.active_form.as_deref() != Some(active_form) {
                entry.active_form = Some(active_form.to_string());
                updated_fields.push("activeForm");
            }
        }
        if let Some(owner) = input["owner"].as_str() {
            if entry.owner.as_deref() != Some(owner) {
                entry.owner = Some(owner.to_string());
                updated_fields.push("owner");
            }
        }
        if let Some(metadata) = input.get("metadata").filter(|value| value.is_object()) {
            let mut merged = entry
                .metadata
                .as_ref()
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            if let Some(obj) = metadata.as_object() {
                for (key, value) in obj {
                    if value.is_null() {
                        merged.remove(key);
                    } else {
                        merged.insert(key.clone(), value.clone());
                    }
                }
            }
            entry.metadata = Some(Value::Object(merged));
            updated_fields.push("metadata");
        }

        let status_change = if old_status != entry.status {
            json!({
                "from": old_status,
                "to": entry.status,
            })
        } else {
            Value::Null
        };
        save_task_entry(&entry);
        drop(entry);
        if let Some(add_blocks) = input["addBlocks"].as_array() {
            let existing_blocks = load_task_entry(&task_id)
                .map(|task| task.blocks)
                .unwrap_or_default();
            let new_blocks = add_blocks
                .iter()
                .filter_map(Value::as_str)
                .filter(|block_id| !existing_blocks.iter().any(|id| id == block_id))
                .collect::<Vec<_>>();
            for block_id in &new_blocks {
                block_task(&task_id, block_id);
            }
            if !new_blocks.is_empty() {
                updated_fields.push("blocks");
            }
        }
        if let Some(add_blocked_by) = input["addBlockedBy"].as_array() {
            let existing_blocked_by = load_task_entry(&task_id)
                .map(|task| task.blocked_by)
                .unwrap_or_default();
            let new_blocked_by = add_blocked_by
                .iter()
                .filter_map(Value::as_str)
                .filter(|blocker_id| !existing_blocked_by.iter().any(|id| id == blocker_id))
                .collect::<Vec<_>>();
            for blocker_id in &new_blocked_by {
                block_task(blocker_id, &task_id);
            }
            if !new_blocked_by.is_empty() {
                updated_fields.push("blockedBy");
            }
        }
        let mut data = json!({
                "success": true,
                "taskId": task_id,
                "updatedFields": updated_fields,
                "verificationNudgeNeeded": false,
        });
        if !status_change.is_null() {
            data["statusChange"] = status_change;
        }
        Ok(ToolResultData {
            data,
            is_error: false,
        })
    }
}

// ─── TaskGetTool ───────────────────────────────────────────────────────────────

pub struct TaskGetTool;

#[async_trait]
impl ToolExecutor for TaskGetTool {
    fn name(&self) -> &str {
        "TaskGet"
    }

    fn description(&self) -> String {
        TASK_GET_PROMPT.to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "taskId": {
                    "type": "string",
                    "description": "ID of the task to retrieve"
                }
            },
            "required": ["taskId"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }
    fn is_read_only(&self, _input: &Value) -> bool {
        true
    }

    fn to_auto_classifier_input(&self, input: &Value) -> Option<String> {
        Some(input["taskId"].as_str().unwrap_or_default().to_string())
    }

    async fn call(
        &self,
        input: &Value,
        _ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let task_id = match input["taskId"].as_str() {
            Some(id) => id.to_string(),
            None => return Ok(error_result("missing required parameter: taskId")),
        };

        match load_task_entry(&task_id) {
            Some(entry) => Ok(ToolResultData {
                data: json!({
                    "task": {
                        "id": entry.id,
                        "subject": entry.subject,
                        "description": entry.description,
                        "status": entry.status,
                        "blocks": entry.blocks,
                        "blockedBy": entry.blocked_by,
                    }
                }),
                is_error: false,
            }),
            None => Ok(ToolResultData {
                data: json!({ "task": Value::Null }),
                is_error: false,
            }),
        }
    }
}

// ─── TaskStopTool ──────────────────────────────────────────────────────────────

pub struct TaskStopTool;

#[async_trait]
impl ToolExecutor for TaskStopTool {
    fn name(&self) -> &str {
        "TaskStop"
    }

    fn description(&self) -> String {
        "Stop a running task by its ID. If the task has an associated process, kills it."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "taskId": {
                    "type": "string",
                    "description": "ID of the task to stop"
                }
            },
            "required": ["taskId"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }
    fn is_read_only(&self, _input: &Value) -> bool {
        false
    }

    fn to_auto_classifier_input(&self, input: &Value) -> Option<String> {
        Some(
            input["taskId"]
                .as_str()
                .or_else(|| input["task_id"].as_str())
                .or_else(|| input["shell_id"].as_str())
                .unwrap_or_default()
                .to_string(),
        )
    }

    async fn call(
        &self,
        input: &Value,
        _ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let task_id = match input["taskId"].as_str() {
            Some(id) => id.to_string(),
            None => return Ok(error_result("missing required parameter: taskId")),
        };

        // Extract PID before taking the mutable borrow for status update.
        let pid = load_task_entry(&task_id).and_then(|entry| entry.pid);

        // Kill the process if we have a PID.
        let kill_msg: Option<String> = if let Some(pid) = pid {
            kill_process(pid)
        } else {
            None
        };

        match load_task_entry(&task_id) {
            Some(mut entry) => {
                entry.status = "stopped".to_string();
                let mut data = entry.to_json();
                if let Some(msg) = kill_msg {
                    data["killMessage"] = json!(msg);
                }
                save_task_entry(&entry);
                Ok(ToolResultData {
                    data,
                    is_error: false,
                })
            }
            None => Ok(error_result(format!("task not found: {}", task_id))),
        }
    }
}

/// Send SIGKILL (Unix) or TerminateProcess (Windows) on the given PID.
/// Returns a message describing what happened.
fn kill_process(pid: u32) -> Option<String> {
    #[cfg(target_family = "unix")]
    {
        // Use `kill -9` shell command — avoids an FFI dependency on libc.
        let output = std::process::Command::new("kill")
            .args(["-9", &pid.to_string()])
            .output();
        match output {
            Ok(out) if out.status.success() => Some(format!("Sent SIGKILL to PID {}", pid)),
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                Some(format!("kill -9 {} failed: {}", pid, stderr.trim()))
            }
            Err(e) => Some(format!("Failed to invoke kill: {}", e)),
        }
    }
    #[cfg(not(target_family = "unix"))]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .output();
        Some(format!("Attempted to terminate PID {}", pid))
    }
}

// ─── TaskOutputTool ────────────────────────────────────────────────────────────

pub struct TaskOutputTool;

#[async_trait]
impl ToolExecutor for TaskOutputTool {
    fn name(&self) -> &str {
        "TaskOutput"
    }

    fn description(&self) -> String {
        // Port of TS `TaskOutputTool/TaskOutputTool.tsx:172` `prompt()`.
        // The TS tool distinguishes `description()` (one-liner shown
        // in tool lists) from `prompt()` (model-facing guidance);
        // the Rust `ToolExecutor` has only `description()`, so we
        // return the full prompt text here — same pattern as
        // `FileReadTool` (read.rs:325-327).
        "DEPRECATED: Prefer using the Read tool on the task's output file path instead. Background tasks return their output file path in the tool result, and you receive a <task-notification> with the same path when the task completes — Read that file directly.\n\n\
         - Retrieves output from a running or completed task (background shell, agent, or remote session)\n\
         - Takes a task_id parameter identifying the task\n\
         - Returns the task output along with status information\n\
         - Use block=true (default) to wait for task completion\n\
         - Use block=false for non-blocking check of current status\n\
         - Task IDs can be found using the /tasks command\n\
         - Works with all task types: background shells, async agents, and remote sessions"
            .to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "taskId": {
                    "type": "string",
                    "description": "ID of the task to get output for"
                },
                "block": {
                    "type": "boolean",
                    "description": "When true (default), wait for the task to complete before returning. When false, return the current status and partial output without waiting.",
                    "default": true
                }
            },
            "required": ["taskId"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }
    fn is_read_only(&self, _input: &Value) -> bool {
        true
    }

    fn to_auto_classifier_input(&self, input: &Value) -> Option<String> {
        Some(
            input["taskId"]
                .as_str()
                .or_else(|| input["task_id"].as_str())
                .unwrap_or_default()
                .to_string(),
        )
    }

    async fn call(
        &self,
        input: &Value,
        _ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let task_id = match input["taskId"].as_str() {
            Some(id) => id.to_string(),
            None => return Ok(error_result("missing required parameter: taskId")),
        };

        match load_task_entry(&task_id) {
            Some(entry) => {
                // Return real captured output if available, otherwise fall back
                // to the task description so callers always get some context.
                let output = entry
                    .output
                    .clone()
                    .unwrap_or_else(|| entry.description.clone());

                Ok(ToolResultData {
                    data: json!({
                        "taskId": entry.id,
                        "status": entry.status,
                        "output": output,
                        "pid": entry.pid,
                    }),
                    is_error: false,
                })
            }
            None => Ok(error_result(format!("task not found: {}", task_id))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    static ENV_LOCK: StdMutex<()> = StdMutex::new(());

    struct EnvGuard {
        config: Option<std::ffi::OsString>,
        task_list: Option<std::ffi::OsString>,
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.config {
                Some(value) => std::env::set_var("CLAUDE_RS_TEST_TASKS_DIR", value),
                None => std::env::remove_var("CLAUDE_RS_TEST_TASKS_DIR"),
            }
            match &self.task_list {
                Some(value) => std::env::set_var("CLAUDE_CODE_TASK_LIST_ID", value),
                None => std::env::remove_var("CLAUDE_CODE_TASK_LIST_ID"),
            }
        }
    }

    fn set_task_test_env() -> (tempfile::TempDir, EnvGuard) {
        let dir = tempfile::tempdir().unwrap();
        let guard = EnvGuard {
            config: std::env::var_os("CLAUDE_RS_TEST_TASKS_DIR"),
            task_list: std::env::var_os("CLAUDE_CODE_TASK_LIST_ID"),
        };
        std::env::set_var("CLAUDE_RS_TEST_TASKS_DIR", dir.path());
        std::env::set_var("CLAUDE_CODE_TASK_LIST_ID", "task-test");
        if let Ok(mut store) = TASK_STORE.lock() {
            store.clear();
        }
        (dir, guard)
    }

    fn test_ctx() -> ToolUseContext {
        ToolUseContext::for_test(
            std::path::PathBuf::from("/tmp"),
            std::sync::Arc::new(std::sync::Mutex::new(crate::registry::ReadFileState::new())),
            crate::registry::PermissionMode::Default,
        )
    }

    #[test]
    fn task_entries_persist_to_ts_config_task_dir() {
        let _lock = ENV_LOCK.lock().unwrap();
        let (_dir, _guard) = set_task_test_env();
        let entry = TaskEntry {
            id: "1".into(),
            subject: "Persisted".into(),
            description: "On disk".into(),
            active_form: None,
            owner: None,
            status: "pending".into(),
            blocks: Vec::new(),
            blocked_by: Vec::new(),
            metadata: None,
            created_at: "2026-04-29T00:00:00Z".into(),
            output: None,
            pid: None,
        };
        save_task_entry(&entry);
        TASK_STORE.lock().unwrap().clear();

        let loaded = load_task_entry("1").expect("task should load from disk");
        assert_eq!(loaded.subject, "Persisted");
        assert!(task_path("1").ends_with("task-test/1.json"));
    }

    #[test]
    fn task_ids_do_not_reuse_deleted_high_water_values() {
        let _lock = ENV_LOCK.lock().unwrap();
        let (_dir, _guard) = set_task_test_env();
        NEXT_TASK_ID.store(1, Ordering::SeqCst);

        assert_eq!(new_task_id(), "1");
        delete_task_entry("1");
        assert_eq!(new_task_id(), "2");
    }

    #[test]
    fn delete_task_entry_removes_references_like_ts() {
        let _lock = ENV_LOCK.lock().unwrap();
        let (_dir, _guard) = set_task_test_env();

        save_task_entry(&new_task_entry(
            "1".into(),
            "Blocker".into(),
            "Blocks second".into(),
        ));
        save_task_entry(&new_task_entry(
            "2".into(),
            "Blocked".into(),
            "Blocked by first".into(),
        ));
        assert!(block_task("1", "2"));

        delete_task_entry("1");

        assert!(load_task_entry("1").is_none());
        assert_eq!(
            load_task_entry("2").unwrap().blocked_by,
            Vec::<String>::new()
        );
    }

    #[tokio::test]
    async fn task_tools_track_dependency_fields_like_ts() {
        let _lock = ENV_LOCK.lock().unwrap();
        let (_dir, _guard) = set_task_test_env();
        NEXT_TASK_ID.store(1, Ordering::SeqCst);

        let create = TaskCreateTool;
        let first = create
            .call(
                &json!({
                    "subject": "First",
                    "description": "Blocks the second",
                    "activeForm": "Checking",
                    "metadata": {"kind": "setup"}
                }),
                &test_ctx(),
                CancellationToken::new(),
                None,
            )
            .await
            .unwrap();
        let second = create
            .call(
                &json!({
                    "subject": "Second",
                    "description": "Waits for the first"
                }),
                &test_ctx(),
                CancellationToken::new(),
                None,
            )
            .await
            .unwrap();
        let first_id = first.data["task"]["id"].as_str().unwrap();
        let second_id = second.data["task"]["id"].as_str().unwrap();

        let update = TaskUpdateTool;
        let updated = update
            .call(
                &json!({
                    "taskId": first_id,
                    "addBlocks": [second_id],
                    "owner": "agent-a",
                    "metadata": {"kind": null, "priority": "high"}
                }),
                &test_ctx(),
                CancellationToken::new(),
                None,
            )
            .await
            .unwrap();

        assert!(!updated.is_error);
        assert_eq!(
            updated.data["updatedFields"],
            json!(["owner", "metadata", "blocks"])
        );

        let get = TaskGetTool;
        let first_get = get
            .call(
                &json!({ "taskId": first_id }),
                &test_ctx(),
                CancellationToken::new(),
                None,
            )
            .await
            .unwrap();
        let second_get = get
            .call(
                &json!({ "taskId": second_id }),
                &test_ctx(),
                CancellationToken::new(),
                None,
            )
            .await
            .unwrap();
        assert_eq!(first_get.data["task"]["blocks"], json!([second_id]));
        assert_eq!(second_get.data["task"]["blockedBy"], json!([first_id]));

        let persisted = load_task_entry(first_id).unwrap();
        assert_eq!(persisted.active_form.as_deref(), Some("Checking"));
        assert_eq!(persisted.owner.as_deref(), Some("agent-a"));
        assert_eq!(persisted.metadata, Some(json!({"priority": "high"})));
    }

    #[tokio::test]
    async fn task_list_filters_completed_blockers_like_ts() {
        let _lock = ENV_LOCK.lock().unwrap();
        let (_dir, _guard) = set_task_test_env();
        NEXT_TASK_ID.store(1, Ordering::SeqCst);

        let create = TaskCreateTool;
        let blocker = create
            .call(
                &json!({"subject": "Blocker", "description": "Do first"}),
                &test_ctx(),
                CancellationToken::new(),
                None,
            )
            .await
            .unwrap();
        let blocked = create
            .call(
                &json!({"subject": "Blocked", "description": "Do second"}),
                &test_ctx(),
                CancellationToken::new(),
                None,
            )
            .await
            .unwrap();
        let blocker_id = blocker.data["task"]["id"].as_str().unwrap();
        let blocked_id = blocked.data["task"]["id"].as_str().unwrap();

        let update = TaskUpdateTool;
        update
            .call(
                &json!({"taskId": blocked_id, "addBlockedBy": [blocker_id]}),
                &test_ctx(),
                CancellationToken::new(),
                None,
            )
            .await
            .unwrap();

        let list = TaskListTool;
        let before = list
            .call(&json!({}), &test_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();
        let blocked_before = before.data["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|task| task["id"].as_str() == Some(blocked_id))
            .unwrap();
        assert_eq!(blocked_before["blockedBy"], json!([blocker_id]));

        update
            .call(
                &json!({"taskId": blocker_id, "status": "completed"}),
                &test_ctx(),
                CancellationToken::new(),
                None,
            )
            .await
            .unwrap();

        let after = list
            .call(&json!({}), &test_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();
        let blocked_after = after.data["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|task| task["id"].as_str() == Some(blocked_id))
            .unwrap();
        assert_eq!(blocked_after["blockedBy"], json!([]));
    }

    #[tokio::test]
    async fn task_list_hides_internal_metadata_tasks_like_ts() {
        let _lock = ENV_LOCK.lock().unwrap();
        let (_dir, _guard) = set_task_test_env();
        NEXT_TASK_ID.store(1, Ordering::SeqCst);

        let create = TaskCreateTool;
        create
            .call(
                &json!({
                    "subject": "Hidden",
                    "description": "Internal",
                    "metadata": {"_internal": true}
                }),
                &test_ctx(),
                CancellationToken::new(),
                None,
            )
            .await
            .unwrap();
        let visible = create
            .call(
                &json!({"subject": "Visible", "description": "External"}),
                &test_ctx(),
                CancellationToken::new(),
                None,
            )
            .await
            .unwrap();
        let visible_id = visible.data["task"]["id"].as_str().unwrap();

        let list = TaskListTool;
        let result = list
            .call(&json!({}), &test_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();
        let tasks = result.data["tasks"].as_array().unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0]["id"], visible_id);
    }
}
