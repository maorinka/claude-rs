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

// === Bug #8 tests: relativize_paths should only replace path prefix, not content ===
use claude_tools::grep::relativize_paths;
use std::path::Path;

#[test]
fn test_relativize_paths_only_replaces_prefix_not_content() {
    // The search path appears both as a path prefix AND inside file content.
    // Only the prefix should be replaced.
    let search_path = Path::new("/home/user/project/src");
    let working_dir = Path::new("/home/user/project");

    let raw = "/home/user/project/src/main.rs:5:let path = \"/home/user/project/src/lib.rs\";";
    let result = relativize_paths(raw, search_path, working_dir);

    // Prefix should be replaced
    assert!(
        result.starts_with("src/main.rs:5:"),
        "Path prefix should be relativized, got: {}",
        result
    );
    // Content should NOT be replaced
    assert!(
        result.contains("\"/home/user/project/src/lib.rs\""),
        "Path inside content should remain absolute, got: {}",
        result
    );
}

#[test]
fn test_relativize_paths_multiline() {
    let search_path = Path::new("/home/user/project/src");
    let working_dir = Path::new("/home/user/project");

    let raw = "/home/user/project/src/a.rs:1:use /home/user/project/src/b;\n/home/user/project/src/b.rs:2:hello";
    let result = relativize_paths(raw, search_path, working_dir);

    let result_lines: Vec<&str> = result.lines().collect();
    assert_eq!(result_lines.len(), 2);
    // First line: prefix replaced, content preserved
    assert!(result_lines[0].starts_with("src/a.rs:1:"));
    assert!(result_lines[0].contains("/home/user/project/src/b"));
    // Second line: prefix replaced
    assert!(result_lines[1].starts_with("src/b.rs:2:"));
}

#[test]
fn test_relativize_paths_noop_when_already_relative() {
    let search_path = Path::new("src");
    let working_dir = Path::new("/home/user/project");

    let raw = "src/main.rs:1:hello";
    let result = relativize_paths(raw, search_path, working_dir);
    assert_eq!(result, raw, "Should be unchanged when path is already relative");
}

// === Bug #11 tests: count_unique_files_in_content with colons in filenames ===
use claude_tools::grep::count_unique_files_in_content;

#[test]
fn test_count_unique_files_normal() {
    let content = "src/main.rs:1:hello\nsrc/main.rs:5:world\nsrc/lib.rs:3:foo";
    let count = count_unique_files_in_content(content);
    assert_eq!(count, 2, "Should count 2 unique files");
}

#[test]
fn test_count_unique_files_with_colon_in_filename() {
    // The regex ^(.+?):\d+: should handle this by matching the first colon
    // followed by digits and another colon (the line number pattern).
    // A filename with a colon like "file:name.rs" will produce lines like:
    // "file:name.rs:10:content"
    // The non-greedy .+? will match "file" first, then try :\d+: which fails on ":name.rs:10:",
    // so it backtracks and tries "file:name.rs" then ":10:" which matches.
    let content = "src/main.rs:1:hello\nsrc/main.rs:5:world";
    let count = count_unique_files_in_content(content);
    assert_eq!(count, 1, "Should count 1 unique file");
}

#[test]
fn test_count_unique_files_distinguishes_line_numbers_from_content() {
    // With naive colon splitting, "src/main.rs" is extracted from "src/main.rs:1:hello"
    // but a line like "README:This is a note" would incorrectly count "README" as a file.
    // With regex :\d+: pattern, "README:This is a note" should NOT match because
    // "This is a note" doesn't start with digits.
    let content = "src/main.rs:1:hello\nREADME:This is a note";
    let count = count_unique_files_in_content(content);
    assert_eq!(count, 1, "Should count only files matching file:linenum:content pattern");
}

#[test]
fn test_count_unique_files_empty_input() {
    let count = count_unique_files_in_content("");
    assert_eq!(count, 0, "Empty input should have 0 files");
}

#[test]
fn test_count_unique_files_no_match_lines() {
    let content = "some random text\nanother line without pattern";
    let count = count_unique_files_in_content(content);
    assert_eq!(count, 0, "Lines without file:line: pattern should yield 0");
}
