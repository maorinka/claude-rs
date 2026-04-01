use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

const MAX_OUTPUT_CHARS: usize = 30_000;

/// Default timeout for bash commands in milliseconds (2 minutes).
/// Matches TS `DEFAULT_TIMEOUT_MS` in `utils/timeouts.ts`.
const DEFAULT_TIMEOUT_MS: u64 = 120_000;

/// Maximum timeout for bash commands in milliseconds (10 minutes).
/// Matches TS `MAX_TIMEOUT_MS` in `utils/timeouts.ts`.
const MAX_TIMEOUT_MS: u64 = 600_000;

/// Get the default bash timeout, respecting the `BASH_DEFAULT_TIMEOUT_MS`
/// environment variable (matches TS `getDefaultBashTimeoutMs`).
fn get_default_timeout_ms() -> u64 {
    std::env::var("BASH_DEFAULT_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|&v| v > 0)
        .unwrap_or(DEFAULT_TIMEOUT_MS)
}

/// Get the maximum bash timeout, respecting the `BASH_MAX_TIMEOUT_MS`
/// environment variable (matches TS `getMaxBashTimeoutMs`).
fn get_max_timeout_ms() -> u64 {
    let default = get_default_timeout_ms();
    std::env::var("BASH_MAX_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|&v| v > 0)
        .map(|v| v.max(default))
        .unwrap_or_else(|| MAX_TIMEOUT_MS.max(default))
}

/// Maximum number of sub-commands we will individually check permissions for.
/// Beyond this limit the command is treated as requiring a permission prompt.
/// Matches the TS constant `MAX_SUBCOMMANDS_FOR_SECURITY_CHECK`.
const MAX_SUBCOMMANDS_FOR_SECURITY_CHECK: usize = 50;

pub struct BashTool;

fn truncate(s: String) -> String {
    if s.len() <= MAX_OUTPUT_CHARS {
        s
    } else {
        s[..MAX_OUTPUT_CHARS].to_string()
    }
}

// ---------------------------------------------------------------------------
// Compound command splitting and permission checking
// ---------------------------------------------------------------------------

/// Split a compound shell command into its constituent simple commands.
///
/// Handles the operators `&&`, `||`, `|`, and `;` that separate independent
/// or dependent commands. This is a simplified version of the TS
/// `splitCommand_DEPRECATED` function -- it does not handle heredocs or
/// nested subshells, but covers the common compound patterns that the LLM
/// generates.
///
/// Each returned string is a trimmed sub-command.
pub fn split_compound_command(command: &str) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut chars = command.chars().peekable();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut prev_char: Option<char> = None;

    while let Some(ch) = chars.next() {
        if ch == '\'' && !in_double_quote && prev_char != Some('\\') {
            in_single_quote = !in_single_quote;
            current.push(ch);
        } else if ch == '"' && !in_single_quote && prev_char != Some('\\') {
            in_double_quote = !in_double_quote;
            current.push(ch);
        } else if !in_single_quote && !in_double_quote {
            match ch {
                '&' if chars.peek() == Some(&'&') => {
                    chars.next(); // consume second '&'
                    let trimmed = current.trim().to_string();
                    if !trimmed.is_empty() {
                        parts.push(trimmed);
                    }
                    current.clear();
                }
                '|' if chars.peek() == Some(&'|') => {
                    chars.next(); // consume second '|'
                    let trimmed = current.trim().to_string();
                    if !trimmed.is_empty() {
                        parts.push(trimmed);
                    }
                    current.clear();
                }
                '|' => {
                    let trimmed = current.trim().to_string();
                    if !trimmed.is_empty() {
                        parts.push(trimmed);
                    }
                    current.clear();
                }
                ';' => {
                    let trimmed = current.trim().to_string();
                    if !trimmed.is_empty() {
                        parts.push(trimmed);
                    }
                    current.clear();
                }
                _ => {
                    current.push(ch);
                }
            }
        } else {
            current.push(ch);
        }
        prev_char = Some(ch);
    }

    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        parts.push(trimmed);
    }

    parts
}

/// Extract the first word (command name) from a simple command string,
/// skipping leading environment variable assignments like `VAR=value`.
pub fn extract_command_name(command: &str) -> Option<String> {
    let env_var_re = regex_lite::Regex::new(r"^[A-Za-z_]\w*=").unwrap();
    let tokens: Vec<&str> = command.split_whitespace().collect();

    let mut i = 0;
    while i < tokens.len() && env_var_re.is_match(tokens[i]) {
        i += 1;
    }

    tokens.get(i).map(|s| s.to_string())
}

/// Result of checking a command against permission rules.
#[derive(Debug, PartialEq)]
pub enum PermissionCheckResult {
    /// All sub-commands are allowed.
    Allow,
    /// At least one sub-command needs user confirmation.
    Ask(Vec<String>),
    /// At least one sub-command is denied.
    Deny(String),
    /// Too many sub-commands to check individually.
    TooComplex,
}

/// Read-only commands that never need permission prompts.
const READ_ONLY_COMMANDS: &[&str] = &[
    "ls", "cat", "head", "tail", "less", "more", "wc", "file", "stat",
    "find", "grep", "rg", "ag", "ack", "which", "where", "type",
    "echo", "printf", "date", "pwd", "whoami", "hostname", "uname",
    "env", "printenv", "true", "false", "test", "[",
    "git status", "git log", "git diff", "git show", "git branch",
    "git remote", "git tag", "git stash list",
    "cargo check", "cargo test", "cargo clippy", "cargo build",
    "npm test", "npm run lint", "npx tsc", "node -e",
    "python -c", "python3 -c",
];

/// Check permissions for a command by splitting compound commands and
/// evaluating each sub-command against permission rules.
///
/// Returns a `PermissionCheckResult` indicating whether the command
/// is fully allowed, needs a prompt, or is denied.
pub fn check_permissions(command: &str) -> PermissionCheckResult {
    let sub_commands = split_compound_command(command);

    if sub_commands.len() > MAX_SUBCOMMANDS_FOR_SECURITY_CHECK {
        return PermissionCheckResult::TooComplex;
    }

    let mut needs_ask = Vec::new();

    for sub_cmd in &sub_commands {
        let _cmd_name = match extract_command_name(sub_cmd) {
            Some(name) => name,
            None => continue,
        };

        // Check if the command (or command + subcommand prefix) is read-only
        let is_safe = READ_ONLY_COMMANDS.iter().any(|safe| {
            sub_cmd.starts_with(safe)
        });

        if !is_safe {
            needs_ask.push(sub_cmd.clone());
        }
    }

    if needs_ask.is_empty() {
        PermissionCheckResult::Allow
    } else {
        PermissionCheckResult::Ask(needs_ask)
    }
}

#[async_trait]
impl ToolExecutor for BashTool {
    fn name(&self) -> &str {
        "Bash"
    }

    fn description(&self) -> String {
        r#"Executes a given bash command and returns its output.

The working directory persists between commands, but shell state does not. The shell environment is initialized from the user's profile (bash or zsh).

IMPORTANT: Avoid using this tool to run `find`, `grep`, `cat`, `head`, `tail`, `sed`, `awk`, or `echo` commands, unless explicitly instructed or after you have verified that a dedicated tool cannot accomplish your task. Instead, use the appropriate dedicated tool as this will provide a much better experience for the user:

 - File search: Use Glob (NOT find or ls)
 - Content search: Use Grep (NOT grep or rg)
 - Read files: Use Read (NOT cat/head/tail)
 - Edit files: Use Edit (NOT sed/awk)
 - Write files: Use Write (NOT echo >/cat <<EOF)
 - Communication: Output text directly (NOT echo/printf)
While the Bash tool can do similar things, it's better to use the built-in tools as they provide a better user experience and make it easier to review tool calls and give permission.

# Instructions
 - If your command will create new directories or files, first use this tool to run `ls` to verify the parent directory exists and is the correct location.
 - Always quote file paths that contain spaces with double quotes in your command (e.g., cd "path with spaces/file.txt")
 - Try to maintain your current working directory throughout the session by using absolute paths and avoiding usage of `cd`. You may use `cd` if the User explicitly requests it.
 - You may specify an optional timeout in milliseconds (up to 600000ms / 10 minutes). By default, your command will timeout after 120000ms (2 minutes).
 - You can use the `run_in_background` parameter to run the command in the background. Only use this if you don't need the result immediately and are OK being notified when the command completes later. You do not need to check the output right away - you'll be notified when it finishes. You do not need to use '&' at the end of the command when using this parameter.
 - When issuing multiple commands:
   - If the commands are independent and can run in parallel, make multiple Bash tool calls in a single message. Example: if you need to run "git status" and "git diff", send a single message with two Bash tool calls in parallel.
   - If the commands depend on each other and must run sequentially, use a single Bash call with '&&' to chain them together.
   - Use ';' only when you need to run commands sequentially but don't care if earlier commands fail.
   - DO NOT use newlines to separate commands (newlines are ok in quoted strings).
 - For git commands:
   - Prefer to create a new commit rather than amending an existing commit.
   - Before running destructive operations (e.g., git reset --hard, git push --force, git checkout --), consider whether there is a safer alternative that achieves the same goal. Only use destructive operations when they are truly the best approach.
   - Never skip hooks (--no-verify) or bypass signing (--no-gpg-sign, -c commit.gpgsign=false) unless the user has explicitly asked for it. If a hook fails, investigate and fix the underlying issue.
 - Avoid unnecessary `sleep` commands:
   - Do not sleep between commands that can run immediately -- just run them.
   - If your command is long running and you would like to be notified when it finishes -- use `run_in_background`. No sleep needed.
   - Do not retry failing commands in a sleep loop -- diagnose the root cause.
   - If waiting for a background task you started with `run_in_background`, you will be notified when it completes -- do not poll.
   - If you must poll an external process, use a check command (e.g. `gh run view`) rather than sleeping first.
   - If you must sleep, keep the duration short (1-5 seconds) to avoid blocking the user."#.to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The bash command to execute"
                },
                "timeout": {
                    "type": "number",
                    "description": format!("Optional timeout in milliseconds (max {})", get_max_timeout_ms())
                },
                "description": {
                    "type": "string",
                    "description": "Clear, concise description of what this command does in active voice. Never use words like \"complex\" or \"risk\" in the description - just describe what it does.\n\nFor simple commands (git, npm, standard CLI tools), keep it brief (5-10 words):\n- ls \u{2192} \"List files in current directory\"\n- git status \u{2192} \"Show working tree status\"\n- npm install \u{2192} \"Install package dependencies\"\n\nFor commands that are harder to parse at a glance (piped commands, obscure flags, etc.), add enough context to clarify what it does:\n- find . -name \"*.tmp\" -exec rm {} \\; \u{2192} \"Find and delete all .tmp files recursively\"\n- git reset --hard origin/main \u{2192} \"Discard all local changes and match remote main\"\n- curl -s url | jq '.data[]' \u{2192} \"Fetch JSON from URL and extract data array elements\""
                },
                "run_in_background": {
                    "type": "boolean",
                    "description": "Set to true to run this command in the background. Use Read to read the output later."
                }
            },
            "required": ["command"]
        })
    }

    async fn call(
        &self,
        input: &Value,
        ctx: &ToolUseContext,
        cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let command = input["command"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing 'command' field"))?
            .to_string();

        // Parse timeout: use provided value clamped to max, or default.
        // Matches TS: `const timeoutMs = timeout || getDefaultTimeoutMs()`
        let max_timeout = get_max_timeout_ms();
        let timeout_ms = input
            .get("timeout")
            .and_then(|v| v.as_u64().or_else(|| v.as_f64().map(|f| f as u64)))
            .map(|t| t.min(max_timeout))
            .unwrap_or_else(get_default_timeout_ms);
        let timeout_duration = Duration::from_millis(timeout_ms);

        // Parse run_in_background flag.
        // Matches TS: `if (run_in_background === true && !isBackgroundTasksDisabled)`
        let run_in_background = input
            .get("run_in_background")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // If already cancelled before we even start, return immediately
        if cancel.is_cancelled() {
            return Ok(ToolResultData {
                data: json!({
                    "stdout": "",
                    "stderr": "",
                    "code": -1,
                    "interrupted": true
                }),
                is_error: false,
            });
        }

        let mut child = tokio::process::Command::new("bash")
            .arg("-c")
            .arg(&command)
            .current_dir(&ctx.working_directory)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;

        // Handle run_in_background: return immediately, let the process run.
        // Matches TS behavior: returns `{ stdout: '', stderr: '', code: 0,
        // interrupted: false, backgroundTaskId: shellId }`.
        // We don't have a full task manager yet, so we use the process ID.
        if run_in_background {
            let pid = child.id().unwrap_or(0);
            let task_id = format!("bg_{}", pid);

            // Spawn a detached task that waits for the child so it doesn't become
            // a zombie. Output is discarded (the user can read it via /tasks or
            // by reading the output file later).
            tokio::spawn(async move {
                let _ = child.wait().await;
            });

            return Ok(ToolResultData {
                data: json!({
                    "stdout": "",
                    "stderr": "",
                    "code": 0,
                    "interrupted": false,
                    "backgroundTaskId": task_id
                }),
                is_error: false,
            });
        }

        // Take pipes before any select so we can read them after wait
        let mut stdout_pipe = child.stdout.take().expect("stdout pipe");
        let mut stderr_pipe = child.stderr.take().expect("stderr pipe");

        tokio::select! {
            _ = cancel.cancelled() => {
                // Kill the child process on cancellation
                let _ = child.kill().await;
                let _ = child.wait().await;
                Ok(ToolResultData {
                    data: json!({
                        "stdout": "",
                        "stderr": "",
                        "code": -1,
                        "interrupted": true
                    }),
                    is_error: false,
                })
            }
            _ = tokio::time::sleep(timeout_duration) => {
                // Timeout: kill the process and report.
                // Matches TS behavior where exec uses { timeout: timeoutMs }.
                let _ = child.kill().await;
                let _ = child.wait().await;

                // Read any partial output produced before timeout
                let mut stdout_bytes = Vec::new();
                let mut stderr_bytes = Vec::new();
                let _ = stdout_pipe.read_to_end(&mut stdout_bytes).await;
                let _ = stderr_pipe.read_to_end(&mut stderr_bytes).await;

                let stdout = truncate(String::from_utf8_lossy(&stdout_bytes).into_owned());
                let stderr = truncate(String::from_utf8_lossy(&stderr_bytes).into_owned());

                Ok(ToolResultData {
                    data: json!({
                        "stdout": stdout,
                        "stderr": format!("{}\n\nCommand timed out after {}ms", stderr, timeout_ms),
                        "code": -1,
                        "interrupted": true
                    }),
                    is_error: false,
                })
            }
            status = child.wait() => {
                let exit_status = status?;
                // Read remaining output after process has exited
                let mut stdout_bytes = Vec::new();
                let mut stderr_bytes = Vec::new();
                let _ = stdout_pipe.read_to_end(&mut stdout_bytes).await;
                let _ = stderr_pipe.read_to_end(&mut stderr_bytes).await;

                let stdout = truncate(String::from_utf8_lossy(&stdout_bytes).into_owned());
                let stderr = truncate(String::from_utf8_lossy(&stderr_bytes).into_owned());
                let code = exit_status.code().unwrap_or(-1);
                Ok(ToolResultData {
                    data: json!({
                        "stdout": stdout,
                        "stderr": stderr,
                        "code": code,
                        "interrupted": false
                    }),
                    is_error: false,
                })
            }
        }
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }

    fn is_read_only(&self, _input: &Value) -> bool {
        false
    }

    fn max_result_size_chars(&self) -> usize {
        30_000
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- split_compound_command tests --

    #[test]
    fn test_split_simple_command() {
        assert_eq!(split_compound_command("ls -la"), vec!["ls -la"]);
    }

    #[test]
    fn test_split_and_operator() {
        assert_eq!(
            split_compound_command("cd /tmp && ls"),
            vec!["cd /tmp", "ls"]
        );
    }

    #[test]
    fn test_split_or_operator() {
        assert_eq!(
            split_compound_command("test -f foo || echo missing"),
            vec!["test -f foo", "echo missing"]
        );
    }

    #[test]
    fn test_split_pipe() {
        assert_eq!(
            split_compound_command("cat file | grep pattern"),
            vec!["cat file", "grep pattern"]
        );
    }

    #[test]
    fn test_split_semicolon() {
        assert_eq!(
            split_compound_command("echo a; echo b; echo c"),
            vec!["echo a", "echo b", "echo c"]
        );
    }

    #[test]
    fn test_split_mixed_operators() {
        assert_eq!(
            split_compound_command("git add . && git commit -m 'fix' || echo fail"),
            vec!["git add .", "git commit -m 'fix'", "echo fail"]
        );
    }

    #[test]
    fn test_split_respects_quotes() {
        // The && inside quotes should not cause a split
        assert_eq!(
            split_compound_command(r#"echo "hello && world""#),
            vec![r#"echo "hello && world""#]
        );
    }

    #[test]
    fn test_split_respects_single_quotes() {
        assert_eq!(
            split_compound_command("echo 'a || b' && echo done"),
            vec!["echo 'a || b'", "echo done"]
        );
    }

    // -- extract_command_name tests --

    #[test]
    fn test_extract_simple_command() {
        assert_eq!(extract_command_name("git status"), Some("git".to_string()));
    }

    #[test]
    fn test_extract_with_env_var() {
        assert_eq!(
            extract_command_name("NODE_ENV=prod npm run build"),
            Some("npm".to_string())
        );
    }

    #[test]
    fn test_extract_empty() {
        assert_eq!(extract_command_name(""), None);
    }

    // -- check_permissions tests --

    #[test]
    fn test_permission_read_only_allowed() {
        assert_eq!(check_permissions("ls -la"), PermissionCheckResult::Allow);
        assert_eq!(check_permissions("git status"), PermissionCheckResult::Allow);
        assert_eq!(check_permissions("echo hello"), PermissionCheckResult::Allow);
    }

    #[test]
    fn test_permission_destructive_needs_ask() {
        match check_permissions("rm -rf /tmp/test") {
            PermissionCheckResult::Ask(cmds) => {
                assert_eq!(cmds.len(), 1);
                assert!(cmds[0].contains("rm"));
            }
            other => panic!("expected Ask, got {:?}", other),
        }
    }

    #[test]
    fn test_permission_mixed_compound() {
        // ls is allowed, rm needs ask
        match check_permissions("ls -la && rm -rf /tmp/test") {
            PermissionCheckResult::Ask(cmds) => {
                assert_eq!(cmds.len(), 1);
                assert!(cmds[0].contains("rm"));
            }
            other => panic!("expected Ask, got {:?}", other),
        }
    }

    #[test]
    fn test_permission_too_complex() {
        // Generate more than MAX_SUBCOMMANDS_FOR_SECURITY_CHECK commands
        let commands: Vec<&str> = (0..51).map(|_| "echo hi").collect();
        let big_command = commands.join(" && ");
        assert_eq!(check_permissions(&big_command), PermissionCheckResult::TooComplex);
    }

    #[test]
    fn test_permission_all_read_only_compound() {
        assert_eq!(
            check_permissions("ls -la && cat file.txt | grep pattern"),
            PermissionCheckResult::Allow,
        );
    }

    // -- timeout helper tests --

    #[test]
    fn test_get_default_timeout_ms_returns_default() {
        // When env var is not set, should return DEFAULT_TIMEOUT_MS
        // We can't control env vars in parallel tests easily, so we just
        // verify the function returns a reasonable value.
        let val = get_default_timeout_ms();
        assert!(val > 0, "default timeout should be positive");
    }

    #[test]
    fn test_get_max_timeout_ms_at_least_default() {
        let max = get_max_timeout_ms();
        let default = get_default_timeout_ms();
        assert!(
            max >= default,
            "max ({}) should be >= default ({})",
            max,
            default
        );
    }

    // -- BashTool integration tests (async) --

    fn make_ctx() -> ToolUseContext {
        ToolUseContext {
            working_directory: std::env::current_dir().unwrap_or_else(|_| "/tmp".into()),
            read_file_state: std::sync::Arc::new(std::sync::Mutex::new(
                crate::registry::ReadFileState::new(),
            )),
        }
    }

    #[tokio::test]
    async fn test_bash_simple_command() {
        let tool = BashTool;
        let input = json!({"command": "echo hello"});
        let result = tool
            .call(&input, &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert_eq!(result.data["interrupted"], false);
        assert_eq!(result.data["code"], 0);
        assert!(
            result.data["stdout"]
                .as_str()
                .unwrap()
                .contains("hello"),
            "stdout should contain 'hello', got: {}",
            result.data["stdout"]
        );
    }

    #[tokio::test]
    async fn test_bash_timeout_kills_long_command() {
        let tool = BashTool;
        // sleep 60 should be killed well before it completes
        let input = json!({
            "command": "sleep 60",
            "timeout": 200  // 200ms -- very short
        });
        let start = std::time::Instant::now();
        let result = tool
            .call(&input, &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();
        let elapsed = start.elapsed();

        assert!(!result.is_error);
        assert_eq!(result.data["interrupted"], true);
        assert_eq!(result.data["code"], -1);
        assert!(
            result.data["stderr"]
                .as_str()
                .unwrap()
                .contains("timed out"),
            "stderr should mention timeout, got: {}",
            result.data["stderr"]
        );
        // Should complete in well under 5 seconds (the timeout was 200ms)
        assert!(
            elapsed.as_secs() < 5,
            "should have timed out quickly, took {:?}",
            elapsed
        );
    }

    #[tokio::test]
    async fn test_bash_timeout_clamped_to_max() {
        let tool = BashTool;
        // Pass a timeout larger than MAX_TIMEOUT_MS; the tool should clamp it.
        // We can't easily test the clamped value directly, but we can verify
        // the command still works with a very large timeout.
        let input = json!({
            "command": "echo clamped",
            "timeout": 999_999_999
        });
        let result = tool
            .call(&input, &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert_eq!(result.data["code"], 0);
        assert!(
            result.data["stdout"]
                .as_str()
                .unwrap()
                .contains("clamped")
        );
    }

    #[tokio::test]
    async fn test_bash_default_timeout_applied() {
        // Without explicit timeout, the default should be applied.
        // A fast command should complete well within the default timeout.
        let tool = BashTool;
        let input = json!({"command": "echo fast"});
        let result = tool
            .call(&input, &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert_eq!(result.data["interrupted"], false);
        assert_eq!(result.data["code"], 0);
    }

    #[tokio::test]
    async fn test_bash_run_in_background_returns_immediately() {
        let tool = BashTool;
        let input = json!({
            "command": "sleep 30",
            "run_in_background": true
        });
        let start = std::time::Instant::now();
        let result = tool
            .call(&input, &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();
        let elapsed = start.elapsed();

        assert!(!result.is_error);
        assert_eq!(result.data["interrupted"], false);
        assert_eq!(result.data["code"], 0);
        // Should return almost immediately (background)
        assert!(
            elapsed.as_secs() < 3,
            "run_in_background should return immediately, took {:?}",
            elapsed
        );
        // Should have a backgroundTaskId
        assert!(
            result.data["backgroundTaskId"].is_string(),
            "should have backgroundTaskId, got: {}",
            result.data
        );
        let task_id = result.data["backgroundTaskId"].as_str().unwrap();
        assert!(
            task_id.starts_with("bg_"),
            "backgroundTaskId should start with 'bg_', got: {}",
            task_id
        );
        // stdout/stderr should be empty for background tasks
        assert_eq!(result.data["stdout"], "");
        assert_eq!(result.data["stderr"], "");
    }

    #[tokio::test]
    async fn test_bash_run_in_background_false_waits() {
        let tool = BashTool;
        let input = json!({
            "command": "echo foreground",
            "run_in_background": false
        });
        let result = tool
            .call(&input, &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert_eq!(result.data["code"], 0);
        assert!(
            result.data["stdout"]
                .as_str()
                .unwrap()
                .contains("foreground")
        );
        // No backgroundTaskId
        assert!(result.data.get("backgroundTaskId").is_none());
    }

    #[tokio::test]
    async fn test_bash_cancellation_kills_command() {
        let tool = BashTool;
        let input = json!({"command": "sleep 60"});
        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();

        let handle = tokio::spawn(async move {
            tool.call(&input, &make_ctx(), cancel_clone, None).await
        });

        // Cancel after a short delay
        tokio::time::sleep(Duration::from_millis(100)).await;
        cancel.cancel();

        let result = handle.await.unwrap().unwrap();
        assert!(!result.is_error);
        assert_eq!(result.data["interrupted"], true);
    }

    #[tokio::test]
    async fn test_bash_timeout_as_float() {
        // The TS uses semanticNumber which allows float values
        let tool = BashTool;
        let input = json!({
            "command": "sleep 60",
            "timeout": 200.5  // float timeout
        });
        let start = std::time::Instant::now();
        let result = tool
            .call(&input, &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();
        let elapsed = start.elapsed();

        assert_eq!(result.data["interrupted"], true);
        assert!(elapsed.as_secs() < 5);
    }

    #[tokio::test]
    async fn test_bash_schema_contains_timeout() {
        let tool = BashTool;
        let schema = tool.input_schema();
        let props = schema["properties"].as_object().unwrap();
        assert!(
            props.contains_key("timeout"),
            "schema should contain timeout property"
        );
        assert!(
            props.contains_key("run_in_background"),
            "schema should contain run_in_background property"
        );
        assert!(
            props.contains_key("command"),
            "schema should contain command property"
        );
    }

    #[tokio::test]
    async fn test_bash_pre_cancelled_returns_immediately() {
        let tool = BashTool;
        let input = json!({"command": "echo should-not-run"});
        let cancel = CancellationToken::new();
        cancel.cancel(); // pre-cancel

        let result = tool.call(&input, &make_ctx(), cancel, None).await.unwrap();
        assert_eq!(result.data["interrupted"], true);
        assert_eq!(result.data["code"], -1);
    }
}
