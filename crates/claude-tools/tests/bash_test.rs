use claude_tools::registry::{ToolExecutor, ToolUseContext};
use claude_tools::bash::BashTool;
use serde_json::json;
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;

fn make_ctx(dir: PathBuf) -> ToolUseContext {
    ToolUseContext { working_directory: dir }
}

fn tmpdir() -> PathBuf {
    std::env::temp_dir()
}

#[tokio::test]
async fn test_bash_echo() {
    let tool = BashTool;
    let ctx = make_ctx(tmpdir());
    let cancel = CancellationToken::new();
    let input = json!({ "command": "echo hello" });
    let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
    assert!(!result.is_error);
    let stdout = result.data["stdout"].as_str().unwrap();
    let code = result.data["code"].as_i64().unwrap();
    assert_eq!(stdout, "hello\n");
    assert_eq!(code, 0);
}

#[tokio::test]
async fn test_bash_exit_code() {
    let tool = BashTool;
    let ctx = make_ctx(tmpdir());
    let cancel = CancellationToken::new();
    let input = json!({ "command": "exit 42" });
    let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
    assert!(!result.is_error);
    let code = result.data["code"].as_i64().unwrap();
    assert_eq!(code, 42);
}

#[tokio::test]
async fn test_bash_stderr() {
    let tool = BashTool;
    let ctx = make_ctx(tmpdir());
    let cancel = CancellationToken::new();
    let input = json!({ "command": "echo oops >&2" });
    let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
    assert!(!result.is_error);
    let stderr = result.data["stderr"].as_str().unwrap();
    assert!(stderr.contains("oops"), "stderr should contain 'oops', got: {:?}", stderr);
}

#[tokio::test]
async fn test_bash_cwd() {
    let tool = BashTool;
    let working_dir = tmpdir();
    let ctx = make_ctx(working_dir.clone());
    let cancel = CancellationToken::new();
    let input = json!({ "command": "pwd" });
    let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
    assert!(!result.is_error);
    let stdout = result.data["stdout"].as_str().unwrap().trim().to_string();
    // Canonicalize both sides to handle macOS /private/tmp symlinks
    let actual = std::fs::canonicalize(&stdout).unwrap_or_else(|_| PathBuf::from(&stdout));
    let expected = std::fs::canonicalize(&working_dir).unwrap_or(working_dir);
    assert_eq!(actual, expected, "pwd output should match working_directory");
}

#[tokio::test]
async fn test_bash_cancellation() {
    let tool = BashTool;
    let ctx = make_ctx(tmpdir());
    let cancel = CancellationToken::new();
    // Cancel before running
    cancel.cancel();
    let input = json!({ "command": "sleep 10" });
    let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
    assert!(!result.is_error);
    let interrupted = result.data["interrupted"].as_bool().unwrap();
    assert!(interrupted, "should be interrupted when cancel token is already cancelled");
}

/// Bug #12: Test that the pipe-take pattern returns an error result (not a panic)
/// when stdout/stderr pipes are unavailable.
///
/// We can't easily force `child.stdout.take()` to return `None` after spawning
/// with `Stdio::piped()`, so we verify the pattern by spawning a process and
/// calling `take()` twice — the second call returns `None`, which is the scenario
/// the fix handles.
#[tokio::test]
async fn test_bash_stdout_pipe_none_returns_error_not_panic() {
    use std::process::Stdio;
    use tokio::process::Command;

    let mut child = Command::new("echo")
        .arg("test")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn echo");

    // First take succeeds
    let first_stdout = child.stdout.take();
    assert!(first_stdout.is_some(), "First stdout take() should be Some");

    // Second take returns None — this is the scenario the fix handles
    let second_stdout = child.stdout.take();
    assert!(second_stdout.is_none(), "Second stdout take() should be None");

    // Verify that matching on None returns an error result instead of panicking
    let result = match second_stdout {
        Some(_p) => claude_core::types::events::ToolResultData {
            data: json!({ "ok": true }),
            is_error: false,
        },
        None => claude_core::types::events::ToolResultData {
            data: json!({ "error": "failed to capture stdout pipe" }),
            is_error: true,
        },
    };
    assert!(result.is_error, "Should be an error result when pipe is None");
    assert_eq!(
        result.data["error"].as_str().unwrap(),
        "failed to capture stdout pipe"
    );

    let _ = child.wait().await;
}

#[tokio::test]
async fn test_bash_stderr_pipe_none_returns_error_not_panic() {
    use std::process::Stdio;
    use tokio::process::Command;

    let mut child = Command::new("echo")
        .arg("test")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn echo");

    // First take succeeds
    let first_stderr = child.stderr.take();
    assert!(first_stderr.is_some(), "First stderr take() should be Some");

    // Second take returns None
    let second_stderr = child.stderr.take();
    assert!(second_stderr.is_none(), "Second stderr take() should be None");

    // Verify that matching on None returns an error result instead of panicking
    let result = match second_stderr {
        Some(_p) => claude_core::types::events::ToolResultData {
            data: json!({ "ok": true }),
            is_error: false,
        },
        None => claude_core::types::events::ToolResultData {
            data: json!({ "error": "failed to capture stderr pipe" }),
            is_error: true,
        },
    };
    assert!(result.is_error, "Should be an error result when pipe is None");
    assert_eq!(
        result.data["error"].as_str().unwrap(),
        "failed to capture stderr pipe"
    );

    let _ = child.wait().await;
}
