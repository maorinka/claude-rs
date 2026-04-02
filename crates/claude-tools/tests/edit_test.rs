use claude_core::types::events::ToolResultData;
use claude_tools::edit::FileEditTool;
use claude_tools::registry::{ToolExecutor, ToolUseContext};
use serde_json::json;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

fn make_ctx(dir: &TempDir) -> ToolUseContext {
    ToolUseContext {
        working_directory: dir.path().to_path_buf(),
        read_file_state: std::sync::Arc::new(std::sync::Mutex::new(
            claude_tools::registry::ReadFileState::new(),
        )),
    }
}

async fn call_tool(
    tool: &FileEditTool,
    input: serde_json::Value,
    ctx: &ToolUseContext,
) -> ToolResultData {
    let cancel = CancellationToken::new();
    tool.call(&input, ctx, cancel, None).await.unwrap()
}

#[tokio::test]
async fn test_edit_replace_string() {
    let dir = TempDir::new().unwrap();
    let file_path = dir.path().join("hello.txt");
    std::fs::write(&file_path, "hello world").unwrap();

    let tool = FileEditTool;
    let ctx = make_ctx(&dir);

    // Simulate reading the file first (required by staleness check)
    ctx.read_file_state
        .lock()
        .unwrap()
        .record_read(file_path.to_str().unwrap(), false);

    let result = call_tool(
        &tool,
        json!({
            "file_path": file_path.to_str().unwrap(),
            "old_string": "hello",
            "new_string": "goodbye"
        }),
        &ctx,
    )
    .await;

    assert!(
        !result.is_error,
        "Expected success, got error: {:?}",
        result.data
    );

    let content = std::fs::read_to_string(&file_path).unwrap();
    assert_eq!(content, "goodbye world");

    let data = &result.data;
    assert_eq!(data["filePath"], file_path.to_str().unwrap());
    assert_eq!(data["oldString"], "hello");
    assert_eq!(data["newString"], "goodbye");
    assert!(!data["replaceAll"].as_bool().unwrap_or(true));
}

#[tokio::test]
async fn test_edit_replace_all() {
    let dir = TempDir::new().unwrap();
    let file_path = dir.path().join("foos.txt");
    std::fs::write(&file_path, "foo bar foo baz foo").unwrap();

    let tool = FileEditTool;
    let ctx = make_ctx(&dir);

    // Simulate reading the file first (required by staleness check)
    ctx.read_file_state
        .lock()
        .unwrap()
        .record_read(file_path.to_str().unwrap(), false);

    let result = call_tool(
        &tool,
        json!({
            "file_path": file_path.to_str().unwrap(),
            "old_string": "foo",
            "new_string": "qux",
            "replace_all": true
        }),
        &ctx,
    )
    .await;

    assert!(
        !result.is_error,
        "Expected success, got error: {:?}",
        result.data
    );

    let content = std::fs::read_to_string(&file_path).unwrap();
    assert_eq!(content, "qux bar qux baz qux");

    assert!(result.data["replaceAll"].as_bool().unwrap_or(false));
}

#[tokio::test]
async fn test_edit_error_on_ambiguous_match() {
    let dir = TempDir::new().unwrap();
    let file_path = dir.path().join("multi.txt");
    std::fs::write(&file_path, "foo bar foo").unwrap();

    let tool = FileEditTool;
    let ctx = make_ctx(&dir);

    // Simulate reading the file first (required by staleness check)
    ctx.read_file_state
        .lock()
        .unwrap()
        .record_read(file_path.to_str().unwrap(), false);

    let result = call_tool(
        &tool,
        json!({
            "file_path": file_path.to_str().unwrap(),
            "old_string": "foo",
            "new_string": "baz",
            "replace_all": false
        }),
        &ctx,
    )
    .await;

    assert!(result.is_error, "Expected error for ambiguous match");
}

#[tokio::test]
async fn test_edit_string_not_found() {
    let dir = TempDir::new().unwrap();
    let file_path = dir.path().join("notfound.txt");
    std::fs::write(&file_path, "hello world").unwrap();

    let tool = FileEditTool;
    let ctx = make_ctx(&dir);

    // Simulate reading the file first (required by staleness check)
    ctx.read_file_state
        .lock()
        .unwrap()
        .record_read(file_path.to_str().unwrap(), false);

    let result = call_tool(
        &tool,
        json!({
            "file_path": file_path.to_str().unwrap(),
            "old_string": "missing string",
            "new_string": "replacement"
        }),
        &ctx,
    )
    .await;

    assert!(result.is_error, "Expected error when old_string not found");
}

#[tokio::test]
async fn test_edit_nonexistent_file() {
    let dir = TempDir::new().unwrap();
    let file_path = dir.path().join("does_not_exist.txt");

    let tool = FileEditTool;
    let ctx = make_ctx(&dir);

    let result = call_tool(
        &tool,
        json!({
            "file_path": file_path.to_str().unwrap(),
            "old_string": "hello",
            "new_string": "goodbye"
        }),
        &ctx,
    )
    .await;

    assert!(result.is_error, "Expected error for nonexistent file");
}

#[test]
fn test_edit_is_destructive() {
    let tool = FileEditTool;
    let input = json!({
        "file_path": "/some/path",
        "old_string": "a",
        "new_string": "b"
    });

    assert!(tool.is_destructive(&input));
    assert!(!tool.is_concurrency_safe(&input));
    assert!(!tool.is_read_only(&input));
    assert_eq!(tool.max_result_size_chars(), 100_000);
    assert_eq!(tool.name(), "Edit");
}
