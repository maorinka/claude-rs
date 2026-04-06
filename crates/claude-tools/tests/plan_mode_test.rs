use claude_tools::plan_mode::{
    is_plan_mode_active, set_plan_mode, should_plan_mode_block, EnterPlanModeTool, ExitPlanModeTool,
};
use claude_tools::registry::{ToolExecutor, ToolUseContext};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Mutex;
use tokio_util::sync::CancellationToken;

// Serialise all plan-mode tests since they share the global PLAN_MODE_ACTIVE.
static PLAN_LOCK: Mutex<()> = Mutex::new(());

fn make_ctx() -> ToolUseContext {
    ToolUseContext {
        working_directory: PathBuf::from("/tmp"),
        read_file_state: std::sync::Arc::new(std::sync::Mutex::new(
            claude_tools::registry::ReadFileState::new(),
        )),
        permission_mode: claude_tools::registry::PermissionMode::Default,
    }
}

#[tokio::test]
async fn test_enter_plan_mode_sets_flag() {
    let _guard = PLAN_LOCK.lock().unwrap();
    set_plan_mode(false);
    assert!(!is_plan_mode_active());

    let tool = EnterPlanModeTool;
    let result = tool
        .call(&json!({}), &make_ctx(), CancellationToken::new(), None)
        .await
        .expect("call should not fail");

    assert!(!result.is_error);
    assert_eq!(result.data["mode"], "plan");
    assert!(
        is_plan_mode_active(),
        "plan mode flag should be active after EnterPlanMode"
    );

    set_plan_mode(false);
}

#[tokio::test]
async fn test_exit_plan_mode_clears_flag() {
    let _guard = PLAN_LOCK.lock().unwrap();
    set_plan_mode(true);

    let tool = ExitPlanModeTool;
    let result = tool
        .call(&json!({}), &make_ctx(), CancellationToken::new(), None)
        .await
        .expect("call should not fail");

    assert!(!result.is_error);
    assert_eq!(result.data["mode"], "normal");
    assert!(
        !is_plan_mode_active(),
        "plan mode flag should be inactive after ExitPlanMode"
    );
}

#[tokio::test]
async fn test_exit_plan_mode_errors_when_not_active() {
    let _guard = PLAN_LOCK.lock().unwrap();
    set_plan_mode(false);

    let tool = ExitPlanModeTool;
    let result = tool
        .call(&json!({}), &make_ctx(), CancellationToken::new(), None)
        .await
        .expect("call should not fail");

    assert!(result.is_error, "should error when not in plan mode");
    let msg = result.data["error"].as_str().unwrap();
    assert!(
        msg.contains("not in plan mode"),
        "error should explain not in plan mode, got: {}",
        msg
    );
}

#[test]
fn test_plan_mode_blocks_write_tools() {
    let _guard = PLAN_LOCK.lock().unwrap();
    set_plan_mode(true);

    assert!(should_plan_mode_block("Bash", false));
    assert!(should_plan_mode_block("Write", false));
    assert!(should_plan_mode_block("Edit", false));

    // Read-only tools are NOT blocked
    assert!(!should_plan_mode_block("Read", true));
    assert!(!should_plan_mode_block("Grep", true));
    assert!(!should_plan_mode_block("Glob", true));

    // Plan mode tools themselves are NOT blocked
    assert!(!should_plan_mode_block("EnterPlanMode", true));
    assert!(!should_plan_mode_block("ExitPlanMode", false));
    assert!(!should_plan_mode_block("AskUser", true));

    set_plan_mode(false);
}

#[test]
fn test_plan_mode_inactive_does_not_block() {
    let _guard = PLAN_LOCK.lock().unwrap();
    set_plan_mode(false);

    assert!(!should_plan_mode_block("Bash", false));
    assert!(!should_plan_mode_block("Write", false));
}

#[test]
fn test_enter_plan_mode_is_read_only() {
    let tool = EnterPlanModeTool;
    assert!(tool.is_read_only(&json!({})));
    assert!(tool.is_concurrency_safe(&json!({})));
}

#[test]
fn test_exit_plan_mode_is_not_read_only() {
    let tool = ExitPlanModeTool;
    assert!(!tool.is_read_only(&json!({})));
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

#[tokio::test]
async fn test_enter_exit_roundtrip() {
    let _guard = PLAN_LOCK.lock().unwrap();
    set_plan_mode(false);

    let enter = EnterPlanModeTool;
    let result = enter
        .call(&json!({}), &make_ctx(), CancellationToken::new(), None)
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(is_plan_mode_active());
    assert!(should_plan_mode_block("Bash", false));

    let exit = ExitPlanModeTool;
    let result = exit
        .call(&json!({}), &make_ctx(), CancellationToken::new(), None)
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(!is_plan_mode_active());
    assert!(!should_plan_mode_block("Bash", false));
}
