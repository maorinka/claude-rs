use claude_tools::ask_user::AskUserQuestionTool;
use claude_tools::brief_tool::BriefTool;
use claude_tools::send_message::SendMessageTool;
use claude_tools::lsp_tool::LSPTool;
use claude_tools::registry::{ToolExecutor, ToolUseContext};
use serde_json::json;
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;

fn make_ctx() -> ToolUseContext {
    ToolUseContext {
        working_directory: PathBuf::from("/tmp"),
    }
}

// ── AskUserQuestionTool ────────────────────────────────────────────────────

#[tokio::test]
async fn test_ask_user_basic() {
    let tool = AskUserQuestionTool;
    let result = tool
        .call(
            &json!({ "question": "What is your name?" }),
            &make_ctx(),
            CancellationToken::new(),
            None,
        )
        .await
        .expect("call should not fail");

    assert!(!result.is_error);
    let answer = result.data["answer"].as_str().expect("answer should be a string");
    assert!(answer.contains("What is your name?"), "answer should echo the question");
}

#[tokio::test]
async fn test_ask_user_with_options() {
    let tool = AskUserQuestionTool;
    let result = tool
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
        .expect("call should not fail");

    assert!(!result.is_error);
    let answer = result.data["answer"].as_str().expect("answer should be a string");
    assert!(answer.contains("Pick a color"));
    assert!(answer.contains("red") || answer.contains("green") || answer.contains("blue"),
        "answer should mention the options");
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

// ── BriefTool ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_brief_enable() {
    let tool = BriefTool;
    let result = tool
        .call(&json!({ "enabled": true }), &make_ctx(), CancellationToken::new(), None)
        .await
        .expect("call should not fail");

    assert!(!result.is_error);
    assert_eq!(result.data["brief_mode"], true);
}

#[tokio::test]
async fn test_brief_disable() {
    let tool = BriefTool;
    let result = tool
        .call(&json!({ "enabled": false }), &make_ctx(), CancellationToken::new(), None)
        .await
        .expect("call should not fail");

    assert!(!result.is_error);
    assert_eq!(result.data["brief_mode"], false);
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

// ── SendMessageTool ────────────────────────────────────────────────────────

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
    assert_eq!(result.data["sent"], true);
    assert_eq!(result.data["to"], "agent-2");
}

#[tokio::test]
async fn test_send_message_missing_to() {
    let tool = SendMessageTool;
    let result = tool
        .call(&json!({ "content": "Hello!" }), &make_ctx(), CancellationToken::new(), None)
        .await
        .expect("call should not fail");
    assert!(result.is_error, "missing 'to' should produce an error");
}

#[tokio::test]
async fn test_send_message_missing_content() {
    let tool = SendMessageTool;
    let result = tool
        .call(&json!({ "to": "agent-2" }), &make_ctx(), CancellationToken::new(), None)
        .await
        .expect("call should not fail");
    assert!(result.is_error, "missing 'content' should produce an error");
}

#[test]
fn test_send_message_is_not_read_only() {
    let tool = SendMessageTool;
    assert!(!tool.is_read_only(&json!({ "to": "x", "content": "y" })));
}

