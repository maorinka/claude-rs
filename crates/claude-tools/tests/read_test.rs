use claude_core::types::events::ToolResultData;
use claude_tools::read::FileReadTool;
use claude_tools::registry::{ToolExecutor, ToolUseContext};
use serde_json::{json, Value};
use std::io::Write;
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;

fn make_ctx() -> ToolUseContext {
    ToolUseContext::for_test(PathBuf::from("/tmp"), std::sync::Arc::new(std::sync::Mutex::new(
            claude_tools::registry::ReadFileState::new(),
        )), claude_tools::registry::PermissionMode::Default)
}

async fn call_tool(input: Value) -> ToolResultData {
    let tool = FileReadTool;
    tool.call(&input, &make_ctx(), CancellationToken::new(), None)
        .await
        .expect("call should not return Err")
}

#[tokio::test]
async fn test_read_text_file() {
    let mut f = tempfile::NamedTempFile::new().unwrap();
    writeln!(f, "alpha").unwrap();
    writeln!(f, "beta").unwrap();
    writeln!(f, "gamma").unwrap();
    let path = f.path().to_str().unwrap().to_string();

    let result = call_tool(json!({ "file_path": path })).await;

    assert!(!result.is_error, "should not be an error");

    let file = &result.data["file"];
    assert_eq!(file["numLines"], 3);
    assert_eq!(file["startLine"], 1);
    assert_eq!(file["totalLines"], 3);

    let content = file["content"].as_str().unwrap();
    assert!(
        content.contains("1\talpha"),
        "line 1 should have cat-n format"
    );
    assert!(content.contains("2\tbeta"));
    assert!(content.contains("3\tgamma"));
}

#[tokio::test]
async fn test_read_with_offset_and_limit() {
    let mut f = tempfile::NamedTempFile::new().unwrap();
    for line in &["one", "two", "three", "four", "five"] {
        writeln!(f, "{}", line).unwrap();
    }
    let path = f.path().to_str().unwrap().to_string();

    // offset=2 means skip lines 0 and 1 (0-indexed), so start from line index 2 (1-based: line 3)
    let result = call_tool(json!({ "file_path": path, "offset": 2, "limit": 2 })).await;

    assert!(!result.is_error);

    let file = &result.data["file"];
    assert_eq!(file["numLines"], 2, "should return exactly 2 lines");
    assert_eq!(
        file["startLine"], 3,
        "startLine should be 1-based (offset 2 => line 3)"
    );
    assert_eq!(
        file["totalLines"], 5,
        "totalLines is the full file line count"
    );

    let content = file["content"].as_str().unwrap();
    // lines returned are "three" and "four" (1-based lines 3 and 4)
    assert!(content.contains("3\tthree"), "should contain line 3: three");
    assert!(content.contains("4\tfour"), "should contain line 4: four");
    assert!(!content.contains("one"), "should not contain line 1");
    assert!(!content.contains("five"), "should not contain line 5");
}

#[tokio::test]
async fn test_read_nonexistent_file() {
    let result =
        call_tool(json!({ "file_path": "/tmp/this_file_does_not_exist_abc123.txt" })).await;
    assert!(result.is_error, "missing file should produce is_error=true");
}

#[tokio::test]
async fn test_read_blocked_device() {
    let result = call_tool(json!({ "file_path": "/dev/zero" })).await;
    assert!(
        result.is_error,
        "/dev/zero should be blocked and return is_error=true"
    );
}

#[test]
fn test_read_is_concurrent_and_readonly() {
    let tool = FileReadTool;
    let dummy = json!({});
    assert!(
        tool.is_concurrency_safe(&dummy),
        "FileReadTool should be concurrency-safe"
    );
    assert!(
        tool.is_read_only(&dummy),
        "FileReadTool should be read-only"
    );
    assert_eq!(tool.max_result_size_chars(), usize::MAX);
}
