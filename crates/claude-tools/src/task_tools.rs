use anyhow::Result;
use async_trait::async_trait;
use once_cell::sync::Lazy;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

// ─── Shared state ─────────────────────────────────────────────────────────────

/// A task entry with optional real process tracking.
#[derive(Clone, Debug)]
pub struct TaskEntry {
    pub id: String,
    pub subject: String,
    pub description: String,
    pub status: String, // "pending", "in_progress", "completed", "stopped"
    pub created_at: String,
    /// Captured stdout/stderr output from the background process (if any).
    pub output: Option<String>,
    /// Process ID for background tasks spawned by the system.
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

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn new_task_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn error_result(msg: impl Into<String>) -> ToolResultData {
    ToolResultData {
        data: json!({ "error": msg.into() }),
        is_error: true,
    }
}

// ─── Public helpers for process-tracking integration ──────────────────────────

/// Update a task's output and PID.  Called by the agent/background-spawn
/// infrastructure after it launches a child process.
pub fn register_process(task_id: &str, pid: u32) {
    let mut store = TASK_STORE.lock().unwrap();
    if let Some(entry) = store.get_mut(task_id) {
        entry.pid = Some(pid);
        entry.status = "in_progress".to_string();
    }
}

/// Append captured output to a task entry.
pub fn append_output(task_id: &str, text: &str) {
    let mut store = TASK_STORE.lock().unwrap();
    if let Some(entry) = store.get_mut(task_id) {
        let buf = entry.output.get_or_insert_with(String::new);
        buf.push_str(text);
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
        "Create a new background task with a subject and description.".to_string()
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

    fn is_concurrency_safe(&self, _input: &Value) -> bool { false }
    fn is_read_only(&self, _input: &Value) -> bool { false }

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
            subject,
            description,
            status: "pending".to_string(),
            created_at: now_iso(),
            output: None,
            pid: None,
        };

        let mut store = TASK_STORE.lock().unwrap();
        store.insert(id.clone(), entry.clone());

        Ok(ToolResultData {
            data: entry.to_json(),
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
        "List all tasks, optionally filtered by status.".to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {}
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool { true }
    fn is_read_only(&self, _input: &Value) -> bool { true }

    async fn call(
        &self,
        _input: &Value,
        _ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let store = TASK_STORE.lock().unwrap();
        let mut tasks: Vec<Value> = store.values().map(|t| t.to_json()).collect();
        // Sort by created_at for stable ordering
        tasks.sort_by(|a, b| {
            a["createdAt"].as_str().unwrap_or("").cmp(b["createdAt"].as_str().unwrap_or(""))
        });

        Ok(ToolResultData {
            data: json!({ "tasks": tasks, "count": tasks.len() }),
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
        "Update the status or description of an existing task.".to_string()
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

    fn is_concurrency_safe(&self, _input: &Value) -> bool { false }
    fn is_read_only(&self, _input: &Value) -> bool { false }

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

        let mut store = TASK_STORE.lock().unwrap();
        let entry = match store.get_mut(&task_id) {
            Some(e) => e,
            None => return Ok(error_result(format!("task not found: {}", task_id))),
        };

        if let Some(status) = input["status"].as_str() {
            entry.status = status.to_string();
        }
        if let Some(subject) = input["subject"].as_str() {
            entry.subject = subject.to_string();
        }
        if let Some(description) = input["description"].as_str() {
            entry.description = description.to_string();
        }

        let updated = entry.to_json();
        Ok(ToolResultData {
            data: updated,
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
        "Get the details of a specific task by its ID.".to_string()
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

    fn is_concurrency_safe(&self, _input: &Value) -> bool { true }
    fn is_read_only(&self, _input: &Value) -> bool { true }

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

        let store = TASK_STORE.lock().unwrap();
        match store.get(&task_id) {
            Some(entry) => Ok(ToolResultData {
                data: entry.to_json(),
                is_error: false,
            }),
            None => Ok(error_result(format!("task not found: {}", task_id))),
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

    fn is_concurrency_safe(&self, _input: &Value) -> bool { false }
    fn is_read_only(&self, _input: &Value) -> bool { false }

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
        let pid: Option<u32> = {
            let store = TASK_STORE.lock().unwrap();
            store.get(&task_id).and_then(|e| e.pid)
        };

        // Kill the process if we have a PID.
        let kill_msg: Option<String> = if let Some(pid) = pid {
            kill_process(pid)
        } else {
            None
        };

        let mut store = TASK_STORE.lock().unwrap();
        match store.get_mut(&task_id) {
            Some(entry) => {
                entry.status = "stopped".to_string();
                let mut data = entry.to_json();
                if let Some(msg) = kill_msg {
                    data["killMessage"] = json!(msg);
                }
                Ok(ToolResultData { data, is_error: false })
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
            Ok(out) if out.status.success() => {
                Some(format!("Sent SIGKILL to PID {}", pid))
            }
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
        "Get the output of a completed or running task by its ID.".to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "taskId": {
                    "type": "string",
                    "description": "ID of the task to get output for"
                }
            },
            "required": ["taskId"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool { true }
    fn is_read_only(&self, _input: &Value) -> bool { true }

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

        let store = TASK_STORE.lock().unwrap();
        match store.get(&task_id) {
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
