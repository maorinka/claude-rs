use claude_tools::ask_user::{send_user_answer, AskUserQuestionTool};
use claude_tools::brief_tool::{
    get_brief_system_prompt_section, is_brief_enabled, set_brief_mode, BriefTool,
};
use claude_tools::lsp_tool::LSPTool;
use claude_tools::registry::{ToolExecutor, ToolUseContext};
use claude_tools::send_message::SendMessageTool;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Mutex;
use tokio_util::sync::CancellationToken;

// Serialise channel-flow tests so they don't steal each other's global sender.
static MISC_TEST_LOCK: Mutex<()> = Mutex::new(());
// Serialise brief-mode tests so they don't race on the global BRIEF_MODE flag.
static BRIEF_TEST_LOCK: Mutex<()> = Mutex::new(());

fn make_ctx() -> ToolUseContext {
    ToolUseContext {
        working_directory: PathBuf::from("/tmp"),
        read_file_state: std::sync::Arc::new(std::sync::Mutex::new(
            claude_tools::registry::ReadFileState::new(),
        )),
        permission_mode: claude_tools::registry::PermissionMode::Default,
        ..Default::default()
    }
}

// -- AskUserQuestionTool -------------------------------------------------------

#[tokio::test]
async fn test_ask_user_basic() {
    let _guard = MISC_TEST_LOCK.lock().unwrap();

    let handle = tokio::spawn(async {
        AskUserQuestionTool
            .call(
                &json!({ "question": "What is your name?" }),
                &make_ctx(),
                CancellationToken::new(),
                None,
            )
            .await
            .expect("call should not fail")
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    send_user_answer("Alice".to_string());

    let result = handle.await.unwrap();
    assert!(
        !result.is_error,
        "result should not be an error: {:?}",
        result.data
    );
    let answer = result.data["answer"]
        .as_str()
        .expect("answer should be a string");
    assert_eq!(answer, "Alice", "answer should match what was sent");
    // Check answers map
    assert_eq!(result.data["answers"]["What is your name?"], "Alice");
}

#[tokio::test]
async fn test_ask_user_with_options() {
    let _guard = MISC_TEST_LOCK.lock().unwrap();

    let handle = tokio::spawn(async {
        AskUserQuestionTool
            .call(
                &json!({
                    "question": "Pick a color",
                    "options": ["red", "green", "blue"]
                }),
                &make_ctx(),
                CancellationToken::new(),
                None,
            )
            .await
            .expect("call should not fail")
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    send_user_answer("red".to_string());

    let result = handle.await.unwrap();
    assert!(
        !result.is_error,
        "result should not be an error: {:?}",
        result.data
    );
    let answer = result.data["answer"]
        .as_str()
        .expect("answer should be a string");
    assert_eq!(answer, "red");
}

#[tokio::test]
async fn test_ask_user_missing_question() {
    let tool = AskUserQuestionTool;
    let result = tool
        .call(&json!({}), &make_ctx(), CancellationToken::new(), None)
        .await
        .expect("call should not fail");
    assert!(result.is_error, "missing question should produce an error");
}

#[test]
fn test_ask_user_is_read_only() {
    let tool = AskUserQuestionTool;
    assert!(tool.is_read_only(&json!({})));
}

// -- BriefTool -----------------------------------------------------------------

#[tokio::test]
async fn test_brief_enable() {
    let _guard = BRIEF_TEST_LOCK.lock().unwrap();
    let tool = BriefTool;
    let result = tool
        .call(
            &json!({ "enabled": true }),
            &make_ctx(),
            CancellationToken::new(),
            None,
        )
        .await
        .expect("call should not fail");

    assert!(!result.is_error);
    assert_eq!(result.data["brief_mode"], true);
    assert!(is_brief_enabled());

    // System prompt section should be active
    let section = get_brief_system_prompt_section();
    assert!(
        section.is_some(),
        "should have brief system prompt section when enabled"
    );
    assert!(section.unwrap().contains("Brief Mode"));

    set_brief_mode(false);
}

#[tokio::test]
async fn test_brief_disable() {
    let _guard = BRIEF_TEST_LOCK.lock().unwrap();
    set_brief_mode(true);
    let tool = BriefTool;
    let result = tool
        .call(
            &json!({ "enabled": false }),
            &make_ctx(),
            CancellationToken::new(),
            None,
        )
        .await
        .expect("call should not fail");

    assert!(!result.is_error);
    assert_eq!(result.data["brief_mode"], false);
    assert!(!is_brief_enabled());

    // System prompt section should be inactive
    let section = get_brief_system_prompt_section();
    assert!(
        section.is_none(),
        "should not have brief system prompt section when disabled"
    );
}

#[tokio::test]
async fn test_brief_missing_enabled() {
    let tool = BriefTool;
    let result = tool
        .call(&json!({}), &make_ctx(), CancellationToken::new(), None)
        .await
        .expect("call should not fail");
    assert!(result.is_error, "missing enabled should produce an error");
}

#[test]
fn test_brief_is_not_read_only() {
    let tool = BriefTool;
    assert!(!tool.is_read_only(&json!({ "enabled": true })));
}

// -- SendMessageTool -----------------------------------------------------------

#[tokio::test]
async fn test_send_message_basic() {
    let tool = SendMessageTool;
    let result = tool
        .call(
            &json!({ "to": "agent-2", "content": "Hello!" }),
            &make_ctx(),
            CancellationToken::new(),
            None,
        )
        .await
        .expect("call should not fail");

    assert!(!result.is_error);
    assert_eq!(result.data["success"], true);
    assert_eq!(result.data["to"], "agent-2");
}

#[tokio::test]
async fn test_send_message_missing_to() {
    let tool = SendMessageTool;
    let result = tool
        .call(
            &json!({ "content": "Hello!" }),
            &make_ctx(),
            CancellationToken::new(),
            None,
        )
        .await
        .expect("call should not fail");
    assert!(result.is_error, "missing 'to' should produce an error");
}

#[tokio::test]
async fn test_send_message_missing_content() {
    let tool = SendMessageTool;
    let result = tool
        .call(
            &json!({ "to": "agent-2" }),
            &make_ctx(),
            CancellationToken::new(),
            None,
        )
        .await
        .expect("call should not fail");
    assert!(result.is_error, "missing 'content' should produce an error");
}

#[test]
fn test_send_message_is_not_read_only() {
    let tool = SendMessageTool;
    assert!(!tool.is_read_only(&json!({ "to": "x", "content": "y" })));
}

// -- LSPTool -------------------------------------------------------------------

#[tokio::test]
async fn test_lsp_diagnostics_returns_from_manager() {
    let tool = LSPTool;
    let result = tool
        .call(
            &json!({ "operation": "diagnostics", "filePath": "/nonexistent/file.rs" }),
            &make_ctx(),
            CancellationToken::new(),
            None,
        )
        .await
        .expect("call should not fail");

    assert!(!result.is_error);
    assert_eq!(result.data["operation"], "diagnostics");
    assert_eq!(result.data["resultCount"], 0);
}

#[tokio::test]
async fn test_lsp_unknown_operation() {
    let tool = LSPTool;
    let result = tool
        .call(
            &json!({ "operation": "badOp", "filePath": "/test.rs" }),
            &make_ctx(),
            CancellationToken::new(),
            None,
        )
        .await
        .expect("call should not fail");

    assert!(result.is_error);
    assert!(result.data["error"]
        .as_str()
        .unwrap()
        .contains("Unknown LSP operation"));
}

#[test]
fn test_lsp_is_read_only() {
    let tool = LSPTool;
    assert!(tool.is_read_only(&json!({})));
    assert!(tool.is_concurrency_safe(&json!({})));
}
