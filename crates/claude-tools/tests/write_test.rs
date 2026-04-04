use claude_tools::registry::{ToolExecutor, ToolUseContext};
use claude_tools::write::FileWriteTool;
use claude_core::types::events::ToolResultData;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

fn make_ctx(dir: &std::path::Path) -> ToolUseContext {
    ToolUseContext {
        working_directory: dir.to_path_buf(),
    }
}

async fn call_tool(tool: &FileWriteTool, input: Value, ctx: &ToolUseContext) -> ToolResultData {
    tool.call(&input, ctx, CancellationToken::new(), None)
        .await
        .expect("tool call should succeed")
}

#[tokio::test]
async fn test_write_new_file() {
    let tmp = tempfile::tempdir().unwrap();
    let tool = FileWriteTool;
    let ctx = make_ctx(tmp.path());
    let file_path = tmp.path().join("new_file.txt").to_string_lossy().to_string();
    let content = "hello, world!";

    let result = call_tool(
        &tool,
        json!({ "file_path": file_path, "content": content }),
        &ctx,
    )
    .await;

    assert!(!result.is_error);
    assert_eq!(result.data["type"], "create");
    assert_eq!(result.data["filePath"], file_path);
    assert_eq!(result.data["content"], content);
    assert!(result.data["originalFile"].is_null());

    let on_disk = std::fs::read_to_string(&file_path).unwrap();
    assert_eq!(on_disk, content);
}

#[tokio::test]
async fn test_write_creates_parent_dirs() {
    let tmp = tempfile::tempdir().unwrap();
    let tool = FileWriteTool;
    let ctx = make_ctx(tmp.path());
    let file_path = tmp
        .path()
        .join("a/b/c/deep.txt")
        .to_string_lossy()
        .to_string();
    let content = "deep content";

    let result = call_tool(
        &tool,
        json!({ "file_path": file_path, "content": content }),
        &ctx,
    )
    .await;

    assert!(!result.is_error);
    assert_eq!(result.data["type"], "create");

    let on_disk = std::fs::read_to_string(&file_path).unwrap();
    assert_eq!(on_disk, content);
    assert!(tmp.path().join("a/b/c").is_dir());
}

#[tokio::test]
async fn test_write_overwrites_existing() {
    let tmp = tempfile::tempdir().unwrap();
    let tool = FileWriteTool;
    let ctx = make_ctx(tmp.path());
    let file_path = tmp
        .path()
        .join("existing.txt")
        .to_string_lossy()
        .to_string();
    let original = "original content";
    let new_content = "updated content";

    // Create the file first
    std::fs::write(&file_path, original).unwrap();

    let result = call_tool(
        &tool,
        json!({ "file_path": file_path, "content": new_content }),
        &ctx,
    )
    .await;

    assert!(!result.is_error);
    assert_eq!(result.data["type"], "update");
    assert_eq!(result.data["content"], new_content);
    assert_eq!(result.data["originalFile"], original);

    let on_disk = std::fs::read_to_string(&file_path).unwrap();
    assert_eq!(on_disk, new_content);
}

#[tokio::test]
async fn test_write_is_destructive() {
    let tool = FileWriteTool;
    let input = json!({ "file_path": "/tmp/x.txt", "content": "y" });

    assert!(tool.is_destructive(&input));
    assert!(!tool.is_concurrency_safe(&input));
    assert!(!tool.is_read_only(&input));
    assert_eq!(tool.max_result_size_chars(), 100_000);
    assert_eq!(tool.name(), "Write");
}

/// Bug #20: Write should be atomic — no leftover .tmp files after successful write.
#[tokio::test]
async fn test_write_atomic_no_leftover_tmp() {
    let tmp = tempfile::tempdir().unwrap();
    let tool = FileWriteTool;
    let ctx = make_ctx(tmp.path());
    let file_path = tmp.path().join("atomic_test.txt").to_string_lossy().to_string();
    let content = "atomic content";

    let result = call_tool(
        &tool,
        json!({ "file_path": file_path, "content": content }),
        &ctx,
    )
    .await;

    assert!(!result.is_error);

    // The final file should exist with correct content
    let on_disk = std::fs::read_to_string(&file_path).unwrap();
    assert_eq!(on_disk, content);

    // No leftover .tmp file should remain
    let tmp_path = std::path::Path::new(&file_path).with_extension("tmp");
    assert!(
        !tmp_path.exists(),
        "Temporary file should not remain after atomic write: {:?}",
        tmp_path
    );
}

/// Bug #20: Atomic write should preserve content on overwrite.
#[tokio::test]
async fn test_write_atomic_overwrite_is_complete() {
    let tmp = tempfile::tempdir().unwrap();
    let tool = FileWriteTool;
    let ctx = make_ctx(tmp.path());
    let file_path = tmp.path().join("overwrite_test.txt").to_string_lossy().to_string();

    // Write initial content
    std::fs::write(&file_path, "initial content").unwrap();

    // Overwrite with new content atomically
    let new_content = "new content that replaces the old";
    let result = call_tool(
        &tool,
        json!({ "file_path": file_path, "content": new_content }),
        &ctx,
    )
    .await;

    assert!(!result.is_error);

    // Verify the content is exactly the new content (not partial or corrupted)
    let on_disk = std::fs::read_to_string(&file_path).unwrap();
    assert_eq!(on_disk, new_content);

    // No leftover .tmp file
    let tmp_path = std::path::Path::new(&file_path).with_extension("tmp");
    assert!(!tmp_path.exists());
}
