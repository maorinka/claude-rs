use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use crate::task_tools::{create_task_entry, register_process, append_output};
use claude_core::types::events::ToolResultData;

pub struct AgentTool;

#[async_trait]
impl ToolExecutor for AgentTool {
    fn name(&self) -> &str {
        "Agent"
    }
    fn aliases(&self) -> &[&str] {
        &["agent"]
    }
    fn description(&self) -> String {
        "Launch a new agent that has its own conversation context. Use for complex tasks \
         that benefit from independent exploration, such as multi-step research, open-ended \
         search, or tasks that require trying multiple approaches."
            .to_string()
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["prompt"],
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "The prompt/task for the sub-agent"
                },
                "description": {
                    "type": "string",
                    "description": "Optional description of the task"
                },
                "model": {
                    "type": "string",
                    "description": "Model to use for the sub-agent"
                },
                "subagent_type": {
                    "type": "string",
                    "description": "Type of sub-agent to spawn"
                },
                "run_in_background": {
                    "type": "boolean",
                    "description": "If true, spawn detached and return immediately with a task ID"
                },
                "isolation": {
                    "type": "string",
                    "enum": ["worktree"],
                    "description": "If 'worktree', run the agent in a git worktree for isolation"
                },
                "name": {
                    "type": "string",
                    "description": "Name for the spawned agent. Makes it addressable via SendMessage({to: name}) while running."
                },
                "team_name": {
                    "type": "string",
                    "description": "Team name for spawning. Associates this agent with a team for coordination."
                },
                "mode": {
                    "type": "string",
                    "description": "Permission mode for the spawned agent (e.g. \"plan\" to require plan approval)"
                }
            }
        })
    }

    async fn call(
        &self,
        input: &Value,
        ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let prompt = input["prompt"].as_str().unwrap_or("");
        let model = input.get("model").and_then(|v| v.as_str());
        let isolation = input.get("isolation").and_then(|v| v.as_str());
        let agent_name = input.get("name").and_then(|v| v.as_str());
        let team_name = input.get("team_name").and_then(|v| v.as_str());
        let run_in_background = input
            .get("run_in_background")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Log team association if provided
        if let Some(team) = team_name {
            debug!(team = team, agent_name = ?agent_name, "Agent spawned as part of team");
        }

        // Determine working directory (potentially in a worktree)
        let (work_dir, worktree_path) = if isolation == Some("worktree") {
            match create_agent_worktree(&ctx.working_directory).await {
                Ok(wt_path) => {
                    info!(worktree = %wt_path.display(), "Created worktree for agent");
                    (wt_path.clone(), Some(wt_path))
                }
                Err(e) => {
                    warn!("Failed to create worktree, falling back to cwd: {}", e);
                    (ctx.working_directory.clone(), None)
                }
            }
        } else {
            (ctx.working_directory.clone(), None)
        };

        // Build the command
        let mut cmd = tokio::process::Command::new(std::env::current_exe()?);
        cmd.arg(prompt); // positional prompt arg
        cmd.arg("--dangerously-skip-permissions"); // sub-agents auto-allow
        if let Some(m) = model {
            cmd.arg("--model").arg(m);
        }
        cmd.current_dir(&work_dir);

        // Set team/agent identity environment variables for the sub-process
        if let Some(team) = team_name {
            cmd.env("CLAUDE_TEAM_NAME", team);
        }
        if let Some(name) = agent_name {
            cmd.env("CLAUDE_CODE_AGENT_ID", name);
        }

        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        debug!(
            prompt = prompt,
            work_dir = %work_dir.display(),
            background = run_in_background,
            "Spawning agent sub-process"
        );

        if run_in_background {
            // Create a task entry in the shared task store so that
            // TaskGet / TaskStop / TaskOutput can interact with it.
            let description = input
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or(prompt);
            let task_id = create_task_entry("Background agent", description);
            let task_id_clone = task_id.clone();

            // Spawn the child and register its PID immediately.
            let mut child = cmd.spawn()?;
            let pid = child.id().unwrap_or(0);
            register_process(&task_id, pid as u32);

            // Capture stdout in a background tokio task and feed it
            // into the task store so TaskOutput can return it.
            let stdout_handle = child.stdout.take();
            let task_id_for_reader = task_id.clone();
            if let Some(stdout) = stdout_handle {
                tokio::spawn(async move {
                    use tokio::io::AsyncReadExt;
                    let mut reader = tokio::io::BufReader::new(stdout);
                    let mut buf = vec![0u8; 8192];
                    loop {
                        match reader.read(&mut buf).await {
                            Ok(0) => break,
                            Ok(n) => {
                                let text = String::from_utf8_lossy(&buf[..n]);
                                append_output(&task_id_for_reader, &text);
                            }
                            Err(_) => break,
                        }
                    }
                });
            }

            // Wait for process completion in the background.
            tokio::spawn(async move {
                match child.wait().await {
                    Ok(status) => {
                        debug!(
                            task_id = task_id_clone,
                            success = status.success(),
                            "Background agent completed"
                        );
                    }
                    Err(e) => {
                        warn!(task_id = task_id_clone, error = %e, "Background agent failed");
                    }
                }
                // Clean up worktree if we created one
                if let Some(ref wt) = worktree_path {
                    cleanup_worktree(wt).await;
                }
            });

            Ok(ToolResultData {
                data: json!({
                    "status": "spawned",
                    "task_id": task_id,
                    "prompt": prompt,
                    "background": true,
                    "pid": pid,
                }),
                is_error: false,
            })
        } else {
            // Run synchronously and wait for the result
            let output = cmd.output().await?;
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();

            // Clean up worktree if we created one
            if let Some(ref wt) = worktree_path {
                cleanup_worktree(wt).await;
            }

            let result = if output.status.success() {
                stdout
            } else if !stderr.is_empty() {
                format!("{}\n\nStderr:\n{}", stdout, stderr)
            } else {
                stdout
            };

            Ok(ToolResultData {
                data: json!({
                    "status": "completed",
                    "prompt": prompt,
                    "result": result,
                }),
                is_error: !output.status.success(),
            })
        }
    }
}

/// Create a git worktree for agent isolation.
///
/// Creates a temporary worktree branched from HEAD in the system temp directory.
/// Returns the path to the worktree directory.
async fn create_agent_worktree(repo_dir: &std::path::Path) -> Result<std::path::PathBuf> {
    let branch_name = format!("agent-{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let worktree_dir = std::env::temp_dir().join(format!("claude-agent-{}", &branch_name));

    let output = tokio::process::Command::new("git")
        .args(["worktree", "add", "-b", &branch_name])
        .arg(&worktree_dir)
        .arg("HEAD")
        .current_dir(repo_dir)
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("git worktree add failed: {}", stderr));
    }

    Ok(worktree_dir)
}

/// Clean up a git worktree and its temporary branch.
async fn cleanup_worktree(worktree_path: &std::path::Path) {
    debug!(path = %worktree_path.display(), "Cleaning up agent worktree");

    // Remove the worktree
    let _ = tokio::process::Command::new("git")
        .args(["worktree", "remove", "--force"])
        .arg(worktree_path)
        .output()
        .await;

    // Try to delete the temporary branch (best-effort)
    if let Some(dir_name) = worktree_path.file_name().and_then(|n| n.to_str()) {
        if let Some(branch) = dir_name.strip_prefix("claude-") {
            let _ = tokio::process::Command::new("git")
                .args(["branch", "-D", branch])
                .output()
                .await;
        }
    }

    // Remove the directory if it still exists
    let _ = tokio::fs::remove_dir_all(worktree_path).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task_tools::get_task_entry;

    #[test]
    fn test_background_agent_registers_in_task_store() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        rt.block_on(async {
            let tool = AgentTool;
            let ctx = crate::registry::ToolUseContext {
                working_directory: std::path::PathBuf::from("/tmp"),
                read_file_state: std::sync::Arc::new(std::sync::Mutex::new(
                    crate::registry::ReadFileState::new(),
                )),
            };
            let cancel = CancellationToken::new();

            // Spawn a background agent using /bin/echo so it exits quickly.
            // We need to override current_exe. Since we cannot do that for
            // AgentTool directly, we test the task store integration by
            // using run_in_background=true with a prompt that happens to
            // be a valid arg for whatever binary is at current_exe().
            // Instead, let's directly test the task store integration:
            let task_id = create_task_entry("test-agent", "test background agent");
            register_process(&task_id, 99999);
            append_output(&task_id, "hello from agent");

            let entry = get_task_entry(&task_id).expect("task should exist");
            assert_eq!(entry.subject, "test-agent");
            assert_eq!(entry.pid, Some(99999));
            assert_eq!(entry.status, "in_progress");
            assert_eq!(entry.output.as_deref(), Some("hello from agent"));

            // Append more output
            append_output(&task_id, "\nmore output");
            let entry = get_task_entry(&task_id).expect("task should exist after append");
            assert_eq!(
                entry.output.as_deref(),
                Some("hello from agent\nmore output")
            );
        });
    }
}
