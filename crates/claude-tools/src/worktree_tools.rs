use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

// ─── EnterWorktreeTool ───────────────────────────────────────────────────────

pub struct EnterWorktreeTool;

#[async_trait]
impl ToolExecutor for EnterWorktreeTool {
    fn name(&self) -> &str {
        "EnterWorktree"
    }

    fn description(&self) -> String {
        r#"Use this tool ONLY when the user explicitly asks to work in a worktree. This tool creates an isolated git worktree and switches the current session into it.

## When to Use

- The user explicitly says "worktree" (e.g., "start a worktree", "work in a worktree", "create a worktree")

## When NOT to Use

- The user asks to create a branch, switch branches, or work on a different branch -- use git commands instead
- The user asks to fix a bug or work on a feature -- use normal git workflow unless they specifically mention worktrees

## Requirements

- Must be in a git repository
- Must not already be in a worktree

## Behavior

- Creates a new git worktree inside `.claude/worktrees/` with a new branch based on HEAD
- Switches the session's working directory to the new worktree
- Use ExitWorktree to leave the worktree mid-session

## Parameters

- `name` (optional): A name for the worktree. If not provided, a random name is generated."#
            .to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Optional name for the worktree. Each segment may contain only letters, digits, dots, underscores, and dashes; max 64 chars total."
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
        let name = input.get("name").and_then(|v| v.as_str());

        // Validate name if provided
        if let Some(n) = name {
            if n.len() > 64 {
                return Ok(ToolResultData {
                    data: json!({ "error": "Worktree name must be at most 64 characters" }),
                    is_error: true,
                });
            }
            // Validate each segment
            for segment in n.split('/') {
                if segment.is_empty() {
                    return Ok(ToolResultData {
                        data: json!({ "error": "Worktree name segments must not be empty" }),
                        is_error: true,
                    });
                }
                if !segment
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '.' || c == '_' || c == '-')
                {
                    return Ok(ToolResultData {
                        data: json!({ "error": format!("Invalid segment '{}': only letters, digits, dots, underscores, and dashes allowed", segment) }),
                        is_error: true,
                    });
                }
            }
        }

        let cwd = &ctx.working_directory;

        // Check if we are in a git repository
        let git_check = tokio::process::Command::new("git")
            .args(["rev-parse", "--git-dir"])
            .current_dir(cwd)
            .output()
            .await;

        match git_check {
            Ok(output) if output.status.success() => {}
            _ => {
                return Ok(ToolResultData {
                    data: json!({ "error": "Not in a git repository. EnterWorktree requires a git repository." }),
                    is_error: true,
                });
            }
        }

        // Find the canonical git root
        let root_output = tokio::process::Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .current_dir(cwd)
            .output()
            .await?;

        let git_root = String::from_utf8_lossy(&root_output.stdout)
            .trim()
            .to_string();

        // Generate worktree name
        let slug = match name {
            Some(n) => n.to_string(),
            None => format!("wt-{}", &uuid::Uuid::new_v4().to_string()[..8]),
        };

        let worktree_dir = format!("{}/.claude/worktrees/{}", git_root, slug);
        let branch_name = format!("worktree/{}", slug);

        // Create the worktree
        let result = tokio::process::Command::new("git")
            .args(["worktree", "add", "-b", &branch_name, &worktree_dir])
            .current_dir(&git_root)
            .output()
            .await;

        match result {
            Ok(output) if output.status.success() => Ok(ToolResultData {
                data: json!({
                    "worktreePath": worktree_dir,
                    "worktreeBranch": branch_name,
                    "message": format!(
                        "Created worktree at {} on branch {}. The session is now working in the worktree. Use ExitWorktree to leave mid-session.",
                        worktree_dir, branch_name
                    )
                }),
                is_error: false,
            }),
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                Ok(ToolResultData {
                    data: json!({ "error": format!("Failed to create worktree: {}", stderr) }),
                    is_error: true,
                })
            }
            Err(e) => Ok(ToolResultData {
                data: json!({ "error": format!("Failed to run git worktree add: {}", e) }),
                is_error: true,
            }),
        }
    }
}

// ─── ExitWorktreeTool ────────────────────────────────────────────────────────

pub struct ExitWorktreeTool;

#[async_trait]
impl ToolExecutor for ExitWorktreeTool {
    fn name(&self) -> &str {
        "ExitWorktree"
    }

    fn description(&self) -> String {
        r#"Exit a worktree session created by EnterWorktree and return the session to the original working directory.

## Scope

This tool ONLY operates on worktrees created by EnterWorktree in this session. It will NOT touch:
- Worktrees you created manually with `git worktree add`
- Worktrees from a previous session

## Parameters

- `action` (required): `"keep"` or `"remove"`
  - `"keep"` -- leave the worktree and branch intact on disk.
  - `"remove"` -- delete the worktree directory and its branch.
- `discard_changes` (optional, default false): only meaningful with `action: "remove"`. If the worktree has uncommitted changes, the tool will refuse unless this is true."#
            .to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["keep", "remove"],
                    "description": "\"keep\" leaves the worktree on disk; \"remove\" deletes it."
                },
                "discard_changes": {
                    "type": "boolean",
                    "description": "Required true when action is \"remove\" and the worktree has uncommitted changes."
                }
            },
            "required": ["action"]
        })
    }

    fn is_destructive(&self, input: &Value) -> bool {
        input.get("action").and_then(|v| v.as_str()) == Some("remove")
    }

    async fn call(
        &self,
        input: &Value,
        ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let action = match input.get("action").and_then(|v| v.as_str()) {
            Some(a) => a,
            None => {
                return Ok(ToolResultData {
                    data: json!({ "error": "missing required field: action" }),
                    is_error: true,
                });
            }
        };

        if action != "keep" && action != "remove" {
            return Ok(ToolResultData {
                data: json!({ "error": "action must be \"keep\" or \"remove\"" }),
                is_error: true,
            });
        }

        let discard_changes = input
            .get("discard_changes")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let cwd = &ctx.working_directory;

        // Check if we are in a worktree
        let wt_check = tokio::process::Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .current_dir(cwd)
            .output()
            .await;

        let worktree_path = match wt_check {
            Ok(output) if output.status.success() => {
                String::from_utf8_lossy(&output.stdout).trim().to_string()
            }
            _ => {
                return Ok(ToolResultData {
                    data: json!({ "error": "Not in a git repository or worktree." }),
                    is_error: true,
                });
            }
        };

        // Try to find parent repo via commondir
        let commondir_output = tokio::process::Command::new("git")
            .args(["rev-parse", "--git-common-dir"])
            .current_dir(cwd)
            .output()
            .await?;

        let commondir = String::from_utf8_lossy(&commondir_output.stdout)
            .trim()
            .to_string();

        let git_dir_output = tokio::process::Command::new("git")
            .args(["rev-parse", "--git-dir"])
            .current_dir(cwd)
            .output()
            .await?;

        let git_dir = String::from_utf8_lossy(&git_dir_output.stdout)
            .trim()
            .to_string();

        // If commondir == git_dir, we are in the main repo, not a worktree
        if commondir == git_dir || commondir == "." {
            return Ok(ToolResultData {
                data: json!({ "error": "Not currently in a worktree. This tool only operates on worktrees." }),
                is_error: true,
            });
        }

        if action == "keep" {
            return Ok(ToolResultData {
                data: json!({
                    "action": "keep",
                    "worktreePath": worktree_path,
                    "message": format!(
                        "Exited worktree. Your work is preserved at {}. Session should return to the original directory.",
                        worktree_path
                    )
                }),
                is_error: false,
            });
        }

        // action == "remove"
        // Check for uncommitted changes
        if !discard_changes {
            let status_output = tokio::process::Command::new("git")
                .args(["-C", &worktree_path, "status", "--porcelain"])
                .output()
                .await?;

            let status_text = String::from_utf8_lossy(&status_output.stdout);
            let changed_files: usize = status_text.lines().filter(|l| !l.trim().is_empty()).count();

            if changed_files > 0 {
                return Ok(ToolResultData {
                    data: json!({
                        "error": format!(
                            "Worktree has {} uncommitted {}. Removing will discard this work permanently. Re-invoke with discard_changes: true, or use action: \"keep\".",
                            changed_files,
                            if changed_files == 1 { "file" } else { "files" }
                        )
                    }),
                    is_error: true,
                });
            }
        }

        // Remove the worktree
        let remove_result = tokio::process::Command::new("git")
            .args(["worktree", "remove", "--force", &worktree_path])
            .output()
            .await;

        match remove_result {
            Ok(output) if output.status.success() => Ok(ToolResultData {
                data: json!({
                    "action": "remove",
                    "worktreePath": worktree_path,
                    "message": format!(
                        "Exited and removed worktree at {}. Session should return to the original directory.",
                        worktree_path
                    )
                }),
                is_error: false,
            }),
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                Ok(ToolResultData {
                    data: json!({ "error": format!("Failed to remove worktree: {}", stderr) }),
                    is_error: true,
                })
            }
            Err(e) => Ok(ToolResultData {
                data: json!({ "error": format!("Failed to run git worktree remove: {}", e) }),
                is_error: true,
            }),
        }
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
    async fn enter_worktree_invalid_name_too_long() {
        let tool = EnterWorktreeTool;
        let long_name = "a".repeat(65);
        let input = json!({ "name": long_name });
        let ctx = make_ctx(&PathBuf::from("/tmp"));
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(result.is_error);
        assert!(result.data["error"]
            .as_str()
            .unwrap()
            .contains("at most 64"));
    }

    #[tokio::test]
    async fn enter_worktree_invalid_name_bad_chars() {
        let tool = EnterWorktreeTool;
        let input = json!({ "name": "bad name!" });
        let ctx = make_ctx(&PathBuf::from("/tmp"));
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(result.is_error);
        assert!(result.data["error"]
            .as_str()
            .unwrap()
            .contains("Invalid segment"));
    }

    #[tokio::test]
    async fn enter_worktree_not_a_git_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let tool = EnterWorktreeTool;
        let input = json!({});
        let ctx = make_ctx(tmp.path());
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(result.is_error);
        assert!(result.data["error"]
            .as_str()
            .unwrap()
            .contains("Not in a git repository"));
    }

    #[tokio::test]
    async fn exit_worktree_missing_action() {
        let tool = ExitWorktreeTool;
        let input = json!({});
        let ctx = make_ctx(&PathBuf::from("/tmp"));
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(result.is_error);
        assert!(result.data["error"]
            .as_str()
            .unwrap()
            .contains("missing required field: action"));
    }

    #[tokio::test]
    async fn exit_worktree_invalid_action() {
        let tool = ExitWorktreeTool;
        let input = json!({ "action": "invalid" });
        let ctx = make_ctx(&PathBuf::from("/tmp"));
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(result.is_error);
        assert!(result.data["error"].as_str().unwrap().contains("must be"));
    }

    #[tokio::test]
    async fn exit_worktree_not_in_worktree() {
        let tmp = tempfile::tempdir().unwrap();
        // Init a normal git repo (not a worktree)
        let _ = tokio::process::Command::new("git")
            .args(["init"])
            .current_dir(tmp.path())
            .output()
            .await;

        let tool = ExitWorktreeTool;
        let input = json!({ "action": "keep" });
        let ctx = make_ctx(tmp.path());
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(result.is_error);
    }
}
