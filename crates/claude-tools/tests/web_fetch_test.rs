use claude_tools::registry::{ToolExecutor, ToolUseContext};
use claude_tools::web_fetch::WebFetchTool;
use serde_json::json;
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;

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

#[tokio::test]
async fn test_fetch_nonexistent_url_returns_error_gracefully() {
    let tool = WebFetchTool;
    let input = json!({
        "url": "http://localhost:19999/this-does-not-exist",
        "prompt": "summarise"
    });
    let result = tool
        .call(&input, &make_ctx(), CancellationToken::new(), None)
        .await
        .expect("call should not panic");

    // Should not panic; is_error should be true since connection will be refused
    assert!(
        result.is_error,
        "expected is_error=true for unreachable URL"
    );
}

#[tokio::test]
async fn test_fetch_missing_url_field() {
    let tool = WebFetchTool;
    let input = json!({ "prompt": "summarise" });
    let result = tool
        .call(&input, &make_ctx(), CancellationToken::new(), None)
        .await
        .expect("call should not panic");

    assert!(result.is_error);
    let err_msg = result.data["error"].as_str().unwrap_or("");
    assert!(err_msg.contains("url"), "error should mention 'url'");
}

#[tokio::test]
async fn test_fetch_missing_prompt_field() {
    let tool = WebFetchTool;
    let input = json!({ "url": "http://example.com" });
    let result = tool
        .call(&input, &make_ctx(), CancellationToken::new(), None)
        .await
        .expect("call should not panic");

    assert!(result.is_error);
    let err_msg = result.data["error"].as_str().unwrap_or("");
    assert!(err_msg.contains("prompt"), "error should mention 'prompt'");
}

#[test]
fn test_web_fetch_is_concurrency_safe_and_read_only() {
    let tool = WebFetchTool;
    let input = json!({});
    assert!(tool.is_concurrency_safe(&input));
    assert!(tool.is_read_only(&input));
    assert_eq!(tool.name(), "WebFetch");
}
