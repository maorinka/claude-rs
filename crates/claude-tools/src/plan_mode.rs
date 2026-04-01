use anyhow::Result;
use async_trait::async_trait;
use once_cell::sync::Lazy;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

// ---------------------------------------------------------------------------
// Global plan-mode state
// ---------------------------------------------------------------------------

/// Process-wide plan mode flag. When active, permission checks should return
/// Ask for ALL tools (preventing execution until ExitPlanMode is called).
///
/// This matches the TS behavior where `toolPermissionContext.mode = 'plan'`
/// causes the permission evaluator to require user confirmation for every tool.
static PLAN_MODE_ACTIVE: Lazy<AtomicBool> = Lazy::new(|| AtomicBool::new(false));

/// Check whether plan mode is currently active.
///
/// When true, all tool executions should be gated behind user confirmation.
/// The permission evaluator uses this to return `Ask` for every tool call
/// except EnterPlanMode/ExitPlanMode themselves.
pub fn is_plan_mode_active() -> bool {
    PLAN_MODE_ACTIVE.load(Ordering::SeqCst)
}

/// Set plan mode state. Called by EnterPlanModeTool and ExitPlanModeTool.
pub fn set_plan_mode(active: bool) {
    PLAN_MODE_ACTIVE.store(active, Ordering::SeqCst);
}

/// Check if a tool should be blocked by plan mode.
///
/// In plan mode, only read-only tools, EnterPlanMode, ExitPlanMode, and
/// AskUser are allowed to proceed. All other tools should require explicit
/// user confirmation (Ask).
pub fn should_plan_mode_block(tool_name: &str, is_read_only: bool) -> bool {
    if !is_plan_mode_active() {
        return false;
    }

    // These tools are always allowed in plan mode
    match tool_name {
        "EnterPlanMode" | "ExitPlanMode" | "AskUser" | "Brief" => false,
        // Read-only tools are allowed (exploration is fine)
        _ if is_read_only => false,
        // Everything else is blocked
        _ => true,
    }
}

// ---------------------------------------------------------------------------
// EnterPlanModeTool
// ---------------------------------------------------------------------------

pub struct EnterPlanModeTool;

#[async_trait]
impl ToolExecutor for EnterPlanModeTool {
    fn name(&self) -> &str {
        "EnterPlanMode"
    }

    fn description(&self) -> String {
        "Switch to plan mode for designing an approach before coding. In plan mode, \
         write operations require explicit approval. You should explore the codebase, \
         identify patterns, and design an implementation strategy. Use ExitPlanMode \
         when ready to present your plan for approval."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    fn is_read_only(&self, _input: &Value) -> bool {
        true
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    async fn call(
        &self,
        _input: &Value,
        _ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        // Activate plan mode globally
        set_plan_mode(true);

        Ok(ToolResultData {
            data: json!({
                "mode": "plan",
                "message": "Entered plan mode. You should now focus on exploring the \
                            codebase and designing an implementation approach.",
                "instructions": "In plan mode, you should:\n\
                    1. Thoroughly explore the codebase to understand existing patterns\n\
                    2. Identify similar features and architectural approaches\n\
                    3. Consider multiple approaches and their trade-offs\n\
                    4. Use AskUser if you need to clarify the approach\n\
                    5. Design a concrete implementation strategy\n\
                    6. When ready, use ExitPlanMode to present your plan for approval\n\n\
                    Remember: DO NOT write or edit any files yet. This is a read-only \
                    exploration and planning phase."
            }),
            is_error: false,
        })
    }
}

// ---------------------------------------------------------------------------
// ExitPlanModeTool
// ---------------------------------------------------------------------------

pub struct ExitPlanModeTool;

#[async_trait]
impl ToolExecutor for ExitPlanModeTool {
    fn name(&self) -> &str {
        "ExitPlanMode"
    }

    fn description(&self) -> String {
        "Exit plan mode and return to normal execution mode. Present your plan \
         for user approval before implementation begins. After approval, tools \
         execute normally."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    fn is_read_only(&self, _input: &Value) -> bool {
        // TS marks this as false since it writes to disk (plan file)
        false
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    async fn call(
        &self,
        _input: &Value,
        _ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        if !is_plan_mode_active() {
            return Ok(ToolResultData {
                data: json!({
                    "error": "You are not in plan mode. This tool is only for exiting plan \
                              mode after writing a plan. If your plan was already approved, \
                              continue with implementation."
                }),
                is_error: true,
            });
        }

        // Deactivate plan mode globally
        set_plan_mode(false);

        Ok(ToolResultData {
            data: json!({
                "mode": "normal",
                "message": "Plan mode exited. User has approved your plan. You can now \
                            proceed with implementation."
            }),
            is_error: false,
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Mutex as StdMutex;

    // Serialise tests that touch the global PLAN_MODE_ACTIVE flag.
    static PLAN_TEST_LOCK: StdMutex<()> = StdMutex::new(());

    fn make_ctx() -> ToolUseContext {
        ToolUseContext {
            working_directory: PathBuf::from("/tmp"),
            read_file_state: std::sync::Arc::new(std::sync::Mutex::new(
                crate::registry::ReadFileState::new(),
            )),
        }
    }

    #[tokio::test]
    async fn test_enter_plan_mode_activates_flag() {
        let _guard = PLAN_TEST_LOCK.lock().unwrap();
        set_plan_mode(false);
        assert!(!is_plan_mode_active());

        let tool = EnterPlanModeTool;
        let result = tool
            .call(&json!({}), &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();

        assert!(!result.is_error);
        assert_eq!(result.data["mode"], "plan");
        assert!(is_plan_mode_active(), "plan mode flag should be active after EnterPlanMode");

        set_plan_mode(false);
    }

    #[tokio::test]
    async fn test_exit_plan_mode_deactivates_flag() {
        let _guard = PLAN_TEST_LOCK.lock().unwrap();
        set_plan_mode(true);
        assert!(is_plan_mode_active());

        let tool = ExitPlanModeTool;
        let result = tool
            .call(&json!({}), &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();

        assert!(!result.is_error);
        assert_eq!(result.data["mode"], "normal");
        assert!(!is_plan_mode_active(), "plan mode flag should be inactive after ExitPlanMode");
    }

    #[tokio::test]
    async fn test_exit_plan_mode_errors_when_not_in_plan_mode() {
        let _guard = PLAN_TEST_LOCK.lock().unwrap();
        set_plan_mode(false);

        let tool = ExitPlanModeTool;
        let result = tool
            .call(&json!({}), &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();

        assert!(result.is_error, "should error when not in plan mode");
        assert!(
            result.data["error"].as_str().unwrap().contains("not in plan mode"),
            "error should explain you're not in plan mode"
        );
    }

    #[test]
    fn test_should_plan_mode_block_write_tools() {
        let _guard = PLAN_TEST_LOCK.lock().unwrap();
        set_plan_mode(true);

        assert!(should_plan_mode_block("Bash", false));
        assert!(should_plan_mode_block("Write", false));
        assert!(should_plan_mode_block("Edit", false));

        assert!(!should_plan_mode_block("Read", true));
        assert!(!should_plan_mode_block("Grep", true));
        assert!(!should_plan_mode_block("Glob", true));
        assert!(!should_plan_mode_block("LSP", true));

        assert!(!should_plan_mode_block("EnterPlanMode", true));
        assert!(!should_plan_mode_block("ExitPlanMode", false));
        assert!(!should_plan_mode_block("AskUser", true));
        assert!(!should_plan_mode_block("Brief", false));

        set_plan_mode(false);
    }

    #[test]
    fn test_plan_mode_does_not_block_when_inactive() {
        let _guard = PLAN_TEST_LOCK.lock().unwrap();
        set_plan_mode(false);

        assert!(!should_plan_mode_block("Bash", false));
        assert!(!should_plan_mode_block("Write", false));
        assert!(!should_plan_mode_block("Edit", false));
    }

    #[tokio::test]
    async fn test_enter_exit_roundtrip() {
        let _guard = PLAN_TEST_LOCK.lock().unwrap();
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

    #[test]
    fn test_enter_plan_mode_properties() {
        let tool = EnterPlanModeTool;
        assert_eq!(tool.name(), "EnterPlanMode");
        assert!(tool.is_read_only(&json!({})));
        assert!(tool.is_concurrency_safe(&json!({})));
    }

    #[test]
    fn test_exit_plan_mode_properties() {
        let tool = ExitPlanModeTool;
        assert_eq!(tool.name(), "ExitPlanMode");
        assert!(!tool.is_read_only(&json!({})));
        assert!(tool.is_concurrency_safe(&json!({})));
    }

    #[tokio::test]
    async fn test_enter_plan_mode_includes_instructions() {
        let _guard = PLAN_TEST_LOCK.lock().unwrap();
        set_plan_mode(false);

        let tool = EnterPlanModeTool;
        let result = tool
            .call(&json!({}), &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();

        let instructions = result.data["instructions"].as_str().unwrap();
        assert!(instructions.contains("explore the codebase"));
        assert!(instructions.contains("ExitPlanMode"));
        assert!(instructions.contains("DO NOT write or edit"));

        set_plan_mode(false);
    }
}
