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
    pub status: String, // "pending", "in_progress", "completed", "stopped"
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
            "status": self.status,
            "createdAt": self.created_at,
            "output": self.output,
            "pid": self.pid,
        })
    }
}

static TASK_STORE: Lazy<Mutex<HashMap<String, TaskEntry>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
static NEXT_TASK_ID: AtomicU64 = AtomicU64::new(1);

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn new_task_id() -> String {
    let disk_max = list_task_entries()
        .into_iter()
        .filter_map(|task| task.id.parse::<u64>().ok())
        .max()
        .unwrap_or(0);
    let minimum = disk_max + 1;
    let candidate = NEXT_TASK_ID
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
            let next = current.max(minimum);
            Some(next + 1)
        })
        .unwrap_or(minimum)
        .max(minimum);
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

fn delete_task_entry(task_id: &str) {
    let _ = std::fs::remove_file(task_path(task_id));
    if let Ok(mut store) = TASK_STORE.lock() {
        store.remove(task_id);
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
    let entry = TaskEntry {
        id: id.clone(),
        subject: subject.to_string(),
        description: description.to_string(),
        status: "pending".to_string(),
        created_at: now_iso(),
        output: None,
        pid: None,
    };
    let mut store = TASK_STORE.lock().unwrap();
    store.insert(id.clone(), entry);
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
        let entry = TaskEntry {
            id: id.clone(),
            subject: subject.clone(),
            description: description.clone(),
            status: "pending".to_string(),
            created_at: now_iso(),
            output: None,
            pid: None,
        };

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
        entries.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        let tasks = entries
            .into_iter()
            .map(|task| {
                json!({
                    "id": task.id,
                    "subject": task.subject,
                    "status": task.status,
                    "owner": Value::Null,
                    "blockedBy": Vec::<String>::new(),
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
                delete_task_entry(&task_id);
                return Ok(ToolResultData {
                    data: json!({
                        "success": true,
                        "taskId": task_id,
                        "updatedFields": ["deleted"],
                        "statusChange": {
                            "from": old_status,
                            "to": "deleted",
                        },
                    }),
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

        let status_change = if old_status != entry.status {
            json!({
                "from": old_status,
                "to": entry.status,
            })
        } else {
            Value::Null
        };
        save_task_entry(&entry);
        Ok(ToolResultData {
            data: json!({
                "success": true,
                "taskId": task_id,
                "updatedFields": updated_fields,
                "statusChange": status_change,
                "verificationNudgeNeeded": false,
            }),
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
                        "blocks": Vec::<String>::new(),
                        "blockedBy": Vec::<String>::new(),
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

    #[test]
    fn task_entries_persist_to_ts_config_task_dir() {
        let _lock = ENV_LOCK.lock().unwrap();
        let (_dir, _guard) = set_task_test_env();
        let entry = TaskEntry {
            id: "1".into(),
            subject: "Persisted".into(),
            description: "On disk".into(),
            status: "pending".into(),
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
}
