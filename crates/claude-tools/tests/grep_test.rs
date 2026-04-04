use claude_tools::grep::GrepTool;
use claude_tools::registry::{ToolExecutor, ToolUseContext};
use serde_json::json;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

fn make_ctx(dir: &TempDir) -> ToolUseContext {
    ToolUseContext {
        working_directory: dir.path().to_path_buf(),
    }
}

/// Write a file relative to the temp dir.
fn write_file(dir: &TempDir, name: &str, content: &str) {
    let path = dir.path().join(name);
    std::fs::write(path, content).expect("failed to write test file");
}

#[tokio::test]
async fn test_grep_finds_pattern() {
    let dir = TempDir::new().unwrap();

    // file1 contains "println" – should match
    write_file(&dir, "file1.rs", "fn main() {\n    println!(\"hello\");\n}\n");
    // file2 does not contain "println"
    write_file(&dir, "file2.rs", "fn add(a: i32, b: i32) -> i32 { a + b }\n");

    let tool = GrepTool;
    let ctx = make_ctx(&dir);
    let input = json!({
        "pattern": "println",
        "path": dir.path().to_str().unwrap()
    });

    let result = tool
        .call(&input, &ctx, CancellationToken::new(), None)
        .await
        .expect("grep call failed");

    assert!(!result.is_error, "expected no error, got: {:?}", result.data);

    let data = &result.data;
    // In files_with_matches mode we get numFiles and filenames
    let num_files = data["numFiles"].as_u64().expect("numFiles missing");
    assert_eq!(num_files, 1, "expected 1 matching file, got {}", num_files);

    let filenames = data["filenames"].as_array().expect("filenames missing");
    assert_eq!(filenames.len(), 1);
    // The filename should mention file1.rs
    let fname = filenames[0].as_str().expect("filename is not a string");
    assert!(
        fname.contains("file1"),
        "expected file1.rs in results, got: {}",
        fname
    );
}

#[tokio::test]
async fn test_grep_content_mode() {
    let dir = TempDir::new().unwrap();

    write_file(&dir, "hello.txt", "hello world\ngoodbye world\nhello again\n");

    let tool = GrepTool;
    let ctx = make_ctx(&dir);
    let input = json!({
        "pattern": "hello",
        "path": dir.path().to_str().unwrap(),
        "output_mode": "content"
    });

    let result = tool
        .call(&input, &ctx, CancellationToken::new(), None)
        .await
        .expect("grep call failed");

    assert!(!result.is_error, "expected no error, got: {:?}", result.data);

    let data = &result.data;
    assert_eq!(data["mode"].as_str(), Some("content"));

    let content = data["content"].as_str().expect("content field missing");
    assert!(
        content.contains("hello world"),
        "expected 'hello world' in content output"
    );
    assert!(
        content.contains("hello again"),
        "expected 'hello again' in content output"
    );

    let num_lines = data["numLines"].as_u64().expect("numLines missing");
    assert!(num_lines >= 2, "expected at least 2 matching lines");
}

#[tokio::test]
async fn test_grep_case_insensitive() {
    let dir = TempDir::new().unwrap();

    write_file(&dir, "mixed.txt", "Hello World\nGOODBYE\nhello again\n");

    let tool = GrepTool;
    let ctx = make_ctx(&dir);
    let input = json!({
        "pattern": "hello",
        "path": dir.path().to_str().unwrap(),
        "output_mode": "content",
        "-i": true
    });

    let result = tool
        .call(&input, &ctx, CancellationToken::new(), None)
        .await
        .expect("grep call failed");

    assert!(!result.is_error, "expected no error, got: {:?}", result.data);

    let content = result.data["content"]
        .as_str()
        .expect("content field missing");

    // Both "Hello World" and "hello again" should appear
    assert!(
        content.contains("Hello World") || content.to_lowercase().contains("hello world"),
        "expected 'Hello World' match with -i flag, got: {}",
        content
    );
    assert!(
        content.contains("hello again"),
        "expected 'hello again' match with -i flag"
    );
    // "GOODBYE" should not appear
    assert!(
        !content.contains("GOODBYE"),
        "GOODBYE should not match 'hello'"
    );
}

#[test]
fn test_grep_is_concurrent_and_readonly() {
    let tool = GrepTool;
    let input = json!({ "pattern": "foo" });
    assert!(
        tool.is_concurrency_safe(&input),
        "GrepTool should be concurrency safe"
    );
    assert!(
        tool.is_read_only(&input),
        "GrepTool should be read only"
    );
}
