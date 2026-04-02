use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

pub struct WorkflowTool;

#[async_trait]
impl ToolExecutor for WorkflowTool {
    fn name(&self) -> &str {
        "Workflow"
    }

    fn description(&self) -> String {
        "Execute a workflow script from the .claude/workflows/ directory. \
         Workflows are predefined automation scripts for common tasks. \
         Use 'list' action to see available workflows, or 'run' to execute one."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["list", "run"], "description": "Whether to list available workflows or run one." },
                "name": { "type": "string", "description": "Name of the workflow to run (required for 'run' action)." },
                "args": { "type": "array", "items": { "type": "string" }, "description": "Arguments to pass to the workflow script." }
            },
            "required": ["action"]
        })
    }

    fn is_read_only(&self, input: &Value) -> bool {
        input
            .get("action")
            .and_then(|v| v.as_str())
            .map(|a| a == "list")
            .unwrap_or(true)
    }
    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }

    async fn call(
        &self,
        input: &Value,
        ctx: &ToolUseContext,
        cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let action = match input.get("action").and_then(|v| v.as_str()) {
            Some(a) => a,
            None => {
                return Ok(ToolResultData {
                    data: json!({ "error": "missing required field: action" }),
                    is_error: true,
                })
            }
        };
        match action {
            "list" => list_workflows(&ctx.working_directory).await,
            "run" => {
                let name = match input.get("name").and_then(|v| v.as_str()) {
                    Some(n) => n,
                    None => {
                        return Ok(ToolResultData {
                            data: json!({ "error": "missing required field: name (required for 'run' action)" }),
                            is_error: true,
                        })
                    }
                };
                let args: Vec<String> = input
                    .get("args")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                run_workflow(&ctx.working_directory, name, &args, cancel).await
            }
            _ => Ok(ToolResultData {
                data: json!({ "error": format!("Unknown action '{}'. Use 'list' or 'run'.", action) }),
                is_error: true,
            }),
        }
    }
}

fn find_workflows_dir(working_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut dir = working_dir.to_path_buf();
    loop {
        let workflows = dir.join(".claude").join("workflows");
        if workflows.is_dir() {
            return Some(workflows);
        }
        if !dir.pop() {
            break;
        }
    }
    None
}

async fn list_workflows(working_dir: &std::path::Path) -> Result<ToolResultData> {
    let workflows_dir = match find_workflows_dir(working_dir) {
        Some(d) => d,
        None => {
            return Ok(ToolResultData {
                data: json!({ "workflows": [], "message": "No .claude/workflows/ directory found in the project." }),
                is_error: false,
            })
        }
    };
    let mut workflows = Vec::new();
    let mut entries = match tokio::fs::read_dir(&workflows_dir).await {
        Ok(e) => e,
        Err(e) => {
            return Ok(ToolResultData {
                data: json!({ "error": format!("Failed to read workflows directory: {}", e) }),
                is_error: true,
            })
        }
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_string())
            .unwrap_or_default();
        workflows
            .push(json!({ "name": name, "path": path.display().to_string(), "extension": ext }));
    }
    workflows.sort_by(|a, b| {
        a["name"]
            .as_str()
            .unwrap_or("")
            .cmp(b["name"].as_str().unwrap_or(""))
    });
    let count = workflows.len();
    Ok(ToolResultData {
        data: json!({ "workflows": workflows, "count": count, "directory": workflows_dir.display().to_string() }),
        is_error: false,
    })
}

async fn run_workflow(
    working_dir: &std::path::Path,
    name: &str,
    args: &[String],
    cancel: CancellationToken,
) -> Result<ToolResultData> {
    if name.contains("..") || name.contains('/') || name.contains('\\') {
        return Ok(ToolResultData {
            data: json!({ "error": "Invalid workflow name: must not contain path separators or '..'" }),
            is_error: true,
        });
    }
    let workflows_dir = match find_workflows_dir(working_dir) {
        Some(d) => d,
        None => {
            return Ok(ToolResultData {
                data: json!({ "error": "No .claude/workflows/ directory found in the project." }),
                is_error: true,
            })
        }
    };
    let script_path = workflows_dir.join(name);
    if !script_path.exists() {
        return Ok(ToolResultData {
            data: json!({ "error": format!("Workflow '{}' not found.", name) }),
            is_error: true,
        });
    }

    let script_str = script_path.to_string_lossy().to_string();
    let mut cmd_args: Vec<String> = vec![script_str];
    cmd_args.extend(args.iter().cloned());

    let mut child_proc = match tokio::process::Command::new("bash")
        .args(&cmd_args)
        .current_dir(working_dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return Ok(ToolResultData {
                data: json!({ "error": format!("Failed to start workflow: {}", e) }),
                is_error: true,
            })
        }
    };

    let stdout_handle = child_proc.stdout.take();
    let stderr_handle = child_proc.stderr.take();

    let wait_result = tokio::select! {
        r = child_proc.wait() => r,
        _ = cancel.cancelled() => {
            let _ = child_proc.kill().await;
            return Ok(ToolResultData { data: json!({ "cancelled": true, "workflow": name, "message": "Workflow execution was cancelled." }), is_error: false });
        }
    };

    match wait_result {
        Ok(status) => {
            let stdout = if let Some(mut h) = stdout_handle {
                use tokio::io::AsyncReadExt;
                let mut buf = Vec::new();
                let _ = h.read_to_end(&mut buf).await;
                String::from_utf8_lossy(&buf).to_string()
            } else {
                String::new()
            };
            let stderr = if let Some(mut h) = stderr_handle {
                use tokio::io::AsyncReadExt;
                let mut buf = Vec::new();
                let _ = h.read_to_end(&mut buf).await;
                String::from_utf8_lossy(&buf).to_string()
            } else {
                String::new()
            };
            let exit_code = status.code().unwrap_or(-1);
            let stdout_trunc = if stdout.len() > 50000 {
                &stdout[..50000]
            } else {
                &stdout
            };
            let stderr_trunc = if stderr.len() > 10000 {
                &stderr[..10000]
            } else {
                &stderr
            };
            Ok(ToolResultData {
                data: json!({ "workflow": name, "exitCode": exit_code, "stdout": stdout_trunc, "stderr": stderr_trunc, "success": status.success() }),
                is_error: !status.success(),
            })
        }
        Err(e) => Ok(ToolResultData {
            data: json!({ "error": format!("Workflow process error: {}", e) }),
            is_error: true,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ReadFileState;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn make_ctx(dir: &std::path::Path) -> ToolUseContext {
        ToolUseContext {
            working_directory: dir.to_path_buf(),
            read_file_state: Arc::new(std::sync::Mutex::new(ReadFileState::new())),
        }
    }

    #[tokio::test]
    async fn workflow_list_no_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let tool = WorkflowTool;
        let result = tool
            .call(
                &json!({ "action": "list" }),
                &make_ctx(tmp.path()),
                CancellationToken::new(),
                None,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.data["workflows"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn workflow_list_with_scripts() {
        let tmp = tempfile::tempdir().unwrap();
        let workflows_dir = tmp.path().join(".claude").join("workflows");
        tokio::fs::create_dir_all(&workflows_dir).await.unwrap();
        tokio::fs::write(
            workflows_dir.join("test-build.sh"),
            "#!/bin/bash\necho hello",
        )
        .await
        .unwrap();
        let tool = WorkflowTool;
        let result = tool
            .call(
                &json!({ "action": "list" }),
                &make_ctx(tmp.path()),
                CancellationToken::new(),
                None,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert_eq!(result.data["workflows"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn workflow_run_path_traversal_blocked() {
        let tmp = tempfile::tempdir().unwrap();
        let tool = WorkflowTool;
        let result = tool
            .call(
                &json!({ "action": "run", "name": "../etc/passwd" }),
                &make_ctx(tmp.path()),
                CancellationToken::new(),
                None,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.data["error"].as_str().unwrap().contains("Invalid"));
    }

    #[tokio::test]
    async fn workflow_tool_properties() {
        let tool = WorkflowTool;
        assert_eq!(tool.name(), "Workflow");
        assert!(tool.is_read_only(&json!({ "action": "list" })));
        assert!(!tool.is_read_only(&json!({ "action": "run" })));
    }
}
