use claude_tools::plan_mode::{EnterPlanModeTool, ExitPlanModeTool};
use claude_tools::registry::{ToolExecutor, ToolUseContext};
use serde_json::json;
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;

fn make_ctx() -> ToolUseContext {
    ToolUseContext {
        working_directory: PathBuf::from("/tmp"),
        read_file_state: std::sync::Arc::new(std::sync::Mutex::new(
            claude_tools::registry::ReadFileState::new(),
        )),
    }
}

#[tokio::test]
async fn test_enter_plan_mode_returns_plan() {
    let tool = EnterPlanModeTool;
    let result = tool
        .call(&json!({}), &make_ctx(), CancellationToken::new(), None)
        .await
        .expect("call should not fail");

    assert!(!result.is_error);
    assert_eq!(result.data["mode"], "plan");
    assert!(
        result.data["message"].as_str().unwrap().contains("plan"),
        "message should mention plan mode"
    );
}

#[tokio::test]
async fn test_exit_plan_mode_returns_normal() {
    let tool = ExitPlanModeTool;
    let result = tool
        .call(&json!({}), &make_ctx(), CancellationToken::new(), None)
        .await
        .expect("call should not fail");

    assert!(!result.is_error);
    assert_eq!(result.data["mode"], "normal");
    let msg = result.data["message"].as_str().unwrap().to_lowercase();
    assert!(
        msg.contains("normal"),
        "message should mention normal mode, got: {}",
        result.data["message"]
    );
}

#[test]
fn test_enter_plan_mode_is_read_only() {
    let tool = EnterPlanModeTool;
    assert!(tool.is_read_only(&json!({})));
    assert!(tool.is_concurrency_safe(&json!({})));
}

#[test]
fn test_exit_plan_mode_is_read_only() {
    let tool = ExitPlanModeTool;
    assert!(tool.is_read_only(&json!({})));
    assert!(tool.is_concurrency_safe(&json!({})));
}

#[test]
fn test_enter_plan_mode_name() {
    let tool = EnterPlanModeTool;
    assert_eq!(tool.name(), "EnterPlanMode");
}

#[test]
fn test_exit_plan_mode_name() {
    let tool = ExitPlanModeTool;
    assert_eq!(tool.name(), "ExitPlanMode");
}
