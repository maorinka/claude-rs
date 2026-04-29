use claude_tools::bash::BashTool;
use claude_tools::registry::{ToolExecutor, ToolUseContext};
use serde_json::json;
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;

fn make_ctx(dir: PathBuf) -> ToolUseContext {
    ToolUseContext::for_test(
        dir,
        std::sync::Arc::new(std::sync::Mutex::new(
            claude_tools::registry::ReadFileState::new(),
        )),
        claude_tools::registry::PermissionMode::Default,
    )
}

fn tmpdir() -> PathBuf {
    std::env::temp_dir()
}

#[tokio::test]
async fn test_bash_echo() {
    let tool = BashTool::new();
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
    let tool = BashTool::new();
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
    let tool = BashTool::new();
    let ctx = make_ctx(tmpdir());
    let cancel = CancellationToken::new();
    let input = json!({ "command": "echo oops >&2" });
    let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
    assert!(!result.is_error);
    let stderr = result.data["stderr"].as_str().unwrap();
    assert!(
        stderr.contains("oops"),
        "stderr should contain 'oops', got: {:?}",
        stderr
    );
}

#[tokio::test]
async fn test_bash_cwd() {
    let tool = BashTool::new();
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
    assert_eq!(
        actual, expected,
        "pwd output should match working_directory"
    );
}

#[tokio::test]
async fn test_bash_cwd_persists_between_foreground_commands() {
    let tool = BashTool::new();
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().join("base");
    let nested = base.join("nested");
    std::fs::create_dir_all(&nested).unwrap();
    let ctx = make_ctx(base);

    let cd_result = tool
        .call(
            &json!({ "command": format!("cd {}", nested.display()) }),
            &ctx,
            CancellationToken::new(),
            None,
        )
        .await
        .unwrap();
    assert!(!cd_result.is_error);
    assert_eq!(cd_result.data["code"], 0);

    let pwd_result = tool
        .call(
            &json!({ "command": "pwd" }),
            &ctx,
            CancellationToken::new(),
            None,
        )
        .await
        .unwrap();
    assert!(!pwd_result.is_error);
    let stdout = pwd_result.data["stdout"].as_str().unwrap().trim();
    let actual = std::fs::canonicalize(stdout).unwrap();
    let expected = std::fs::canonicalize(nested).unwrap();
    assert_eq!(actual, expected);
}

#[tokio::test]
async fn test_bash_cwd_resets_after_cd_outside_allowed_dirs() {
    let tool = BashTool::new();
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().join("base");
    let outside = temp.path().join("outside");
    std::fs::create_dir_all(&base).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let ctx = make_ctx(base.clone());

    let cd_result = tool
        .call(
            &json!({ "command": format!("cd {}", outside.display()) }),
            &ctx,
            CancellationToken::new(),
            None,
        )
        .await
        .unwrap();
    assert!(!cd_result.is_error);
    assert_eq!(cd_result.data["code"], 0);
    assert!(cd_result.data["stderr"]
        .as_str()
        .unwrap()
        .contains("Shell cwd was reset to"));

    let pwd_result = tool
        .call(
            &json!({ "command": "pwd" }),
            &ctx,
            CancellationToken::new(),
            None,
        )
        .await
        .unwrap();
    assert!(!pwd_result.is_error);
    let stdout = pwd_result.data["stdout"].as_str().unwrap().trim();
    let actual = std::fs::canonicalize(stdout).unwrap();
    let expected = std::fs::canonicalize(base).unwrap();
    assert_eq!(actual, expected);
}

#[tokio::test]
async fn test_bash_cancellation() {
    let tool = BashTool::new();
    let ctx = make_ctx(tmpdir());
    let cancel = CancellationToken::new();
    // Cancel before running
    cancel.cancel();
    // Use sleep 1 (< 2s) so detect_blocked_sleep_pattern allows it through.
    let input = json!({ "command": "sleep 1" });
    let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
    assert!(!result.is_error);
    let interrupted = result.data["interrupted"].as_bool().unwrap();
    assert!(
        interrupted,
        "should be interrupted when cancel token is already cancelled"
    );
}
