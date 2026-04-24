#![allow(clippy::await_holding_lock)] // test-only global-state serialization via std::sync::Mutex

use claude_tools::ask_user::{send_user_answer, AskUserQuestionTool};
use claude_tools::registry::{ToolExecutor, ToolUseContext};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Mutex;
use tokio_util::sync::CancellationToken;

// ─── Test serialisation ────────────────────────────────────────────────────────
//
// All channel-flow tests share the global ASK_USER_TX.  If they run in
// parallel they will steal each other's senders.  We serialize them with
// a process-global mutex.

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn make_ctx() -> ToolUseContext {
    ToolUseContext::for_test(
        PathBuf::from("/tmp"),
        std::sync::Arc::new(std::sync::Mutex::new(
            claude_tools::registry::ReadFileState::new(),
        )),
        claude_tools::registry::PermissionMode::Default,
    )
}

/// Verify tool name and schema properties.
#[test]
fn test_ask_user_tool_properties() {
    let tool = AskUserQuestionTool;
    assert_eq!(tool.name(), "AskUser");
    assert!(tool.is_read_only(&json!({})));
    assert!(!tool.is_concurrency_safe(&json!({})));
}

/// Test missing `question` field returns an error result.
#[tokio::test]
async fn test_missing_question_returns_error() {
    let _guard = TEST_LOCK.lock().unwrap();
    let tool = AskUserQuestionTool;
    let input = json!({});
    let result = tool
        .call(&input, &make_ctx(), CancellationToken::new(), None)
        .await
        .unwrap();
    assert!(result.is_error, "missing question should be an error");
    let err = result.data["error"].as_str().unwrap_or("");
    assert!(err.contains("question"), "error should mention 'question'");
}

/// Test the channel-based flow: tool call + send_user_answer.
#[tokio::test]
async fn test_channel_flow_returns_answer() {
    let _guard = TEST_LOCK.lock().unwrap();

    let tool = AskUserQuestionTool;
    let input = json!({ "question": "What is your favourite color?" });
    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();

    // Spawn the tool call — it will block waiting on the channel.
    let handle =
        tokio::spawn(async move { tool.call(&input, &make_ctx(), cancel_clone, None).await });

    // Give the spawned task time to register the sender.
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    // Simulate TUI sending the user's answer.
    let sent = send_user_answer("Blue".to_string());
    assert!(sent, "send_user_answer should find a waiting sender");

    let result = handle
        .await
        .expect("task should not panic")
        .expect("call should succeed");
    assert!(
        !result.is_error,
        "result should not be an error: {:?}",
        result.data
    );
    assert_eq!(
        result.data["answer"].as_str().unwrap_or(""),
        "Blue",
        "answer should match what was sent"
    );
}

/// Test with options list.
#[tokio::test]
async fn test_channel_flow_with_options() {
    let _guard = TEST_LOCK.lock().unwrap();

    let tool = AskUserQuestionTool;
    let input = json!({
        "question": "Pick a number",
        "options": ["1", "2", "3"]
    });
    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();

    let handle =
        tokio::spawn(async move { tool.call(&input, &make_ctx(), cancel_clone, None).await });

    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    send_user_answer("2".to_string());

    let result = handle.await.unwrap().unwrap();
    assert!(
        !result.is_error,
        "result should not be an error: {:?}",
        result.data
    );
    assert_eq!(result.data["answer"].as_str().unwrap_or(""), "2");
}

/// Test cancellation: when the token is cancelled the tool should return an error.
#[tokio::test]
async fn test_cancellation_returns_error() {
    let _guard = TEST_LOCK.lock().unwrap();

    let tool = AskUserQuestionTool;
    let input = json!({ "question": "Will you wait?" });
    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();

    let handle =
        tokio::spawn(async move { tool.call(&input, &make_ctx(), cancel_clone, None).await });

    // Give the task time to register its sender.
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    // Cancel — nobody sends an answer.
    cancel.cancel();

    // Wait for the task to settle.
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    let result = handle.await.unwrap().unwrap();
    assert!(
        result.is_error,
        "cancelled call should be an error: {:?}",
        result.data
    );
    let err = result.data["error"].as_str().unwrap_or("");
    assert!(
        err.contains("cancel"),
        "error should mention cancellation, got: {}",
        err
    );
}
