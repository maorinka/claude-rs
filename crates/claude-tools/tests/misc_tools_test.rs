use claude_tools::ask_user::{send_user_answer, AskUserQuestionTool};
use claude_tools::brief_tool::BriefTool;
use claude_tools::lsp_tool::LSPTool;
use claude_tools::registry::{ToolExecutor, ToolUseContext};
use claude_tools::send_message::SendMessageTool;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Mutex;
use tokio_util::sync::CancellationToken;

// Serialise channel-flow tests so they don't steal each other's global sender.
static MISC_TEST_LOCK: Mutex<()> = Mutex::new(());

fn make_ctx() -> ToolUseContext {
    ToolUseContext {
        working_directory: PathBuf::from("/tmp"),
        read_file_state: std::sync::Arc::new(std::sync::Mutex::new(
            claude_tools::registry::ReadFileState::new(),
        )),
    }
}

// ── AskUserQuestionTool ────────────────────────────────────────────────────
//
// The tool now blocks on a oneshot channel until the TUI sends an answer.
// Tests must spawn the call in a background task and then call send_user_answer.

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
    assert!(!result.is_error, "result should not be an error: {:?}", result.data);
    let answer = result.data["answer"].as_str().expect("answer should be a string");
    assert_eq!(answer, "Alice", "answer should match what was sent");
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
    assert!(!result.is_error, "result should not be an error: {:?}", result.data);
    let answer = result.data["answer"].as_str().expect("answer should be a string");
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

