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

#[derive(Clone, Debug)]
pub struct TaskEntry {
    pub id: String,
    pub subject: String,
    pub description: String,
    pub status: String, // "pending", "in_progress", "completed", "stopped"
    pub created_at: String,
}

impl TaskEntry {
    fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "subject": self.subject,
            "description": self.description,
            "status": self.status,
            "createdAt": self.created_at,
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
        "Stop a running task by its ID.".to_string()
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

        let mut store = TASK_STORE.lock().unwrap();
        match store.get_mut(&task_id) {
            Some(entry) => {
                entry.status = "stopped".to_string();
                let updated = entry.to_json();
                Ok(ToolResultData {
                    data: updated,
                    is_error: false,
                })
            }
            None => Ok(error_result(format!("task not found: {}", task_id))),
        }
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
                // Stub: return the task description as output
                Ok(ToolResultData {
                    data: json!({
                        "taskId": entry.id,
                        "status": entry.status,
                        "output": entry.description,
                    }),
                    is_error: false,
                })
            }
            None => Ok(error_result(format!("task not found: {}", task_id))),
        }
    }
}
