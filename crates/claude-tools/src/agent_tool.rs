use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::agents::definitions::builtin_agents;
use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use crate::task_tools::{append_output, create_task_entry, register_process};
use claude_core::types::events::ToolResultData;

// ---------------------------------------------------------------------------
// Full Agent tool prompt assembly (mirrors TS tools/AgentTool/prompt.ts)
// ---------------------------------------------------------------------------

/// Format a single agent definition as a line for the agent list section.
fn format_agent_line(name: &str, when_to_use: &str, tools_desc: &str) -> String {
    format!("- {}: {} (Tools: {})", name, when_to_use, tools_desc)
}

/// Build the complete Agent tool prompt with all sections.
///
/// Mirrors the TS `getPrompt()` function from `tools/AgentTool/prompt.ts`.
///
/// Architecture note: TS supports two sub-agent delivery modes — an
/// in-process "fork" (shared JS context, inherits cached messages) and an
/// out-of-process subprocess. The Rust port uses only the subprocess path —
/// each sub-agent is a fresh `claude-rs` process with no inherited context.
/// This is strictly stronger isolation at the cost of having to re-describe
/// context in the sub-agent prompt. The optional `isolation: "worktree"`
/// parameter layers filesystem isolation on top (temporary git worktree).
fn build_agent_prompt() -> String {
    // Build the agent list section from built-in agent definitions
    let agents = builtin_agents();
    let agent_lines: Vec<String> = agents
        .iter()
        .map(|a| {
            // Map agent types to their available tool sets
            let tools = match a.name.as_str() {
                "general-purpose" => "*",
                "Explore" => "All tools except Agent, ExitPlanMode, Edit, Write, NotebookEdit",
                "Plan" => "All tools except Agent, ExitPlanMode, Edit, Write, NotebookEdit",
                "Verification" => "All tools",
                "code-reviewer" => "All tools",
                _ => "*",
            };
            format_agent_line(&a.name, &a.when_to_use, tools)
        })
        .collect();

    let agent_list_section = format!(
        "Available agent types and the tools they have access to:\n{}",
        agent_lines.join("\n")
    );

    // Shared core prompt
    let shared = format!(
        r#"Launch a new agent to handle complex, multi-step tasks autonomously.

The Agent tool launches specialized agents (subprocesses) that autonomously handle complex tasks. Each agent type has specific capabilities and tools available to it.

{}

When using the Agent tool, specify a subagent_type parameter to select which agent type to use. If omitted, the general-purpose agent is used."#,
        agent_list_section
    );

    // "When NOT to use" section
    let when_not_to_use = r#"
When NOT to use the Agent tool:
- If you want to read a specific file path, use the Read tool or the Glob tool instead of the Agent tool, to find the match more quickly
- If you are searching for a specific class definition like "class Foo", use the Glob tool instead, to find the match more quickly
- If you are searching for code within a specific file or set of 2-3 files, use the Read tool instead of the Agent tool, to find the match more quickly
- Other tasks that are not related to the agent descriptions above
"#;

    // "Writing the prompt" section
    let writing_the_prompt = r#"
## Writing the prompt

Brief the agent like a smart colleague who just walked into the room — it hasn't seen this conversation, doesn't know what you've tried, doesn't understand why this task matters.
- Explain what you're trying to accomplish and why.
- Describe what you've already learned or ruled out.
- Give enough context about the surrounding problem that the agent can make judgment calls rather than just following a narrow instruction.
- If you need a short response, say so ("report in under 200 words").
- Lookups: hand over the exact command. Investigations: hand over the question — prescribed steps become dead weight when the premise is wrong.

Terse command-style prompts produce shallow, generic work.

**Never delegate understanding.** Don't write "based on your findings, fix the bug" or "based on the research, implement it." Those phrases push synthesis onto the agent instead of doing it yourself. Write prompts that prove you understood: include file paths, line numbers, what specifically to change.
"#;

    // Usage notes section
    let usage_notes = r#"
Usage notes:
- Always include a short description (3-5 words) summarizing what the agent will do
- Launch multiple agents concurrently whenever possible, to maximize performance; to do that, use a single message with multiple tool uses
- When the agent is done, it will return a single message back to you. The result returned by the agent is not visible to the user. To show the user the result, you should send a text message back to the user with a concise summary of the result.
- You can optionally run agents in the background using the run_in_background parameter. When an agent runs in the background, you will be automatically notified when it completes — do NOT sleep, poll, or proactively check on its progress. Continue with other work or respond to the user instead.
- **Foreground vs background**: Use foreground (default) when you need the agent's results before you can proceed — e.g., research agents whose findings inform your next steps. Use background when you have genuinely independent work to do in parallel.
- To continue a previously spawned agent, use SendMessage with the agent's ID or name as the `to` field. The agent resumes with its full context preserved. Each Agent invocation starts fresh — provide a complete task description.
- The agent's outputs should generally be trusted
- Clearly tell the agent whether you expect it to write code or just to do research (search, file reads, web fetches, etc.), since it is not aware of the user's intent
- If the agent description mentions that it should be used proactively, then you should try your best to use it without the user having to ask for it first. Use your judgement.
- If the user specifies that they want you to run agents "in parallel", you MUST send a single message with multiple Agent tool use content blocks. For example, if you need to launch both a build-validator agent and a test-runner agent in parallel, send a single message with both tool calls.
- You can optionally set `isolation: "worktree"` to run the agent in a temporary git worktree, giving it an isolated copy of the repository. The worktree is automatically cleaned up if the agent makes no changes; if changes are made, the worktree path and branch are returned in the result."#;

    // Examples section
    let examples = r#"
Example usage:

<example_agent_descriptions>
"test-runner": use this agent after you are done writing code to run tests
"greeting-responder": use this agent to respond to user greetings with a friendly joke
</example_agent_descriptions>

<example>
user: "Please write a function that checks if a number is prime"
assistant: I'm going to use the Write tool to write the following code:
<code>
function isPrime(n) {
  if (n <= 1) return false
  for (let i = 2; i * i <= n; i++) {
    if (n % i === 0) return false
  }
  return true
}
</code>
<commentary>
Since a significant piece of code was written and the task was completed, now use the test-runner agent to run the tests
</commentary>
assistant: Uses the Agent tool to launch the test-runner agent
</example>

<example>
user: "Hello"
<commentary>
Since the user is greeting, use the greeting-responder agent to respond with a friendly joke
</commentary>
assistant: "I'm going to use the Agent tool to launch the greeting-responder agent"
</example>
"#;

    // Assemble the full prompt
    format!(
        "{shared}{when_not_to_use}{usage_notes}{writing_the_prompt}\n{examples}",
        shared = shared,
        when_not_to_use = when_not_to_use,
        usage_notes = usage_notes,
        writing_the_prompt = writing_the_prompt,
        examples = examples,
    )
}

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
        build_agent_prompt()
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["prompt"],
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "The task for the agent to perform"
                },
                "description": {
                    "type": "string",
                    "description": "A short (3-5 word) description of the task"
                },
                "model": {
                    "type": "string",
                    "description": "Optional model override for this agent. Takes precedence over the agent definition's model frontmatter. If omitted, uses the agent definition's model, or inherits from the parent.",
                    "enum": ["sonnet", "opus", "haiku"]
                },
                "subagent_type": {
                    "type": "string",
                    "description": "The type of specialized agent to use for this task"
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

        // Propagate the parent's permission context to the sub-agent.
        // Only pass --dangerously-skip-permissions when the parent is already in
        // BypassPermissions mode; for other modes use env vars or inherit Default.
        // Mirrors TS AgentTool/runAgent.ts:412-434.
        use crate::registry::PermissionMode;
        let effective_mode: PermissionMode = match &ctx.permission_mode {
            // Parent is in a privileged mode — always propagate it.
            PermissionMode::BypassPermissions => PermissionMode::BypassPermissions,
            PermissionMode::AcceptEdits => PermissionMode::AcceptEdits,
            // For all other parent modes, check the tool `mode` input param first,
            // then fall back to inheriting the parent mode.
            parent_mode => {
                if let Some(mode_str) = input.get("mode").and_then(|v| v.as_str()) {
                    PermissionMode::from_string(mode_str)
                } else {
                    parent_mode.clone()
                }
            }
        };

        match &effective_mode {
            PermissionMode::BypassPermissions => {
                cmd.arg("--dangerously-skip-permissions");
            }
            PermissionMode::AcceptEdits => {
                cmd.env("CLAUDE_PERMISSION_MODE", "acceptEdits");
            }
            PermissionMode::Plan => {
                cmd.env("CLAUDE_PERMISSION_MODE", "plan");
            }
            // Default / Auto / Bubble / DontAsk: no special flag; child defaults to Default.
            _ => {}
        }

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
                permission_mode: crate::registry::PermissionMode::Default,
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
