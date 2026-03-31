use claude_tools::glob_tool::GlobTool;
use claude_tools::registry::{ToolExecutor, ToolUseContext};
use serde_json::json;
use std::fs;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

fn make_ctx(dir: &TempDir) -> ToolUseContext {
    ToolUseContext {
        working_directory: dir.path().to_path_buf(),
    }
}

#[tokio::test]
async fn test_glob_finds_files() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("foo.rs"), "fn foo() {}").unwrap();
    fs::write(dir.path().join("bar.rs"), "fn bar() {}").unwrap();
    fs::write(dir.path().join("readme.md"), "# Hello").unwrap();

    let tool = GlobTool;
    let input = json!({ "pattern": "*.rs" });
    let ctx = make_ctx(&dir);
    let cancel = CancellationToken::new();

    let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
    assert!(!result.is_error, "Expected no error, got: {:?}", result.data);

    let filenames = result.data["filenames"].as_array().unwrap();
    assert_eq!(filenames.len(), 2, "Expected 2 .rs files, got {}", filenames.len());

    // All filenames should end with .rs
    for f in filenames {
        let name = f.as_str().unwrap();
        assert!(name.ends_with(".rs"), "Expected .rs file, got: {name}");
    }

    assert_eq!(result.data["numFiles"].as_u64().unwrap(), 2);
    assert_eq!(result.data["truncated"].as_bool().unwrap(), false);
}

#[tokio::test]
async fn test_glob_with_path_override() {
    let dir = TempDir::new().unwrap();
    let subdir = dir.path().join("subdir");
    fs::create_dir_all(&subdir).unwrap();
    fs::write(subdir.join("alpha.txt"), "alpha").unwrap();
    fs::write(subdir.join("beta.txt"), "beta").unwrap();
    // A file in the parent dir that should NOT be found
    fs::write(dir.path().join("gamma.txt"), "gamma").unwrap();

    let tool = GlobTool;
    let input = json!({
        "pattern": "*.txt",
        "path": subdir.to_str().unwrap()
    });
    let ctx = make_ctx(&dir);
    let cancel = CancellationToken::new();

    let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
    assert!(!result.is_error, "Expected no error, got: {:?}", result.data);

    let filenames = result.data["filenames"].as_array().unwrap();
    assert_eq!(filenames.len(), 2, "Expected 2 files in subdir, got {}", filenames.len());
    assert_eq!(result.data["numFiles"].as_u64().unwrap(), 2);
}

#[tokio::test]
async fn test_glob_returns_truncation_flag() {
    let dir = TempDir::new().unwrap();
    for i in 0..5 {
        fs::write(dir.path().join(format!("file{i}.rs")), format!("// {i}")).unwrap();
    }

    let tool = GlobTool;
    let input = json!({ "pattern": "*.rs" });
    let ctx = make_ctx(&dir);
    let cancel = CancellationToken::new();

    let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
    assert!(!result.is_error);

    let filenames = result.data["filenames"].as_array().unwrap();
    assert_eq!(filenames.len(), 5);
    assert_eq!(result.data["numFiles"].as_u64().unwrap(), 5);
    // 5 is well under 100, so truncated must be false
    assert_eq!(
        result.data["truncated"].as_bool().unwrap(),
        false,
        "5 files should not trigger truncation"
    );
}

#[test]
fn test_glob_is_concurrent_and_readonly() {
    let tool = GlobTool;
    let input = json!({ "pattern": "*.rs" });
    assert!(tool.is_concurrency_safe(&input), "GlobTool should be concurrency-safe");
    assert!(tool.is_read_only(&input), "GlobTool should be read-only");
}

#[test]
fn test_glob_max_result_size() {
    let tool = GlobTool;
    assert_eq!(tool.max_result_size_chars(), 100_000);
}

#[test]
fn test_glob_name() {
    let tool = GlobTool;
    assert_eq!(tool.name(), "Glob");
}

#[tokio::test]
async fn test_glob_invalid_path_returns_error() {
    let dir = TempDir::new().unwrap();
    let tool = GlobTool;
    let input = json!({
        "pattern": "*.rs",
        "path": "/nonexistent/path/that/does/not/exist"
    });
    let ctx = make_ctx(&dir);
    let cancel = CancellationToken::new();

    let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
    assert!(result.is_error, "Expected error for invalid path");
}

#[tokio::test]
async fn test_glob_path_that_is_file_returns_error() {
    let dir = TempDir::new().unwrap();
    let file_path = dir.path().join("somefile.rs");
    fs::write(&file_path, "fn main() {}").unwrap();

    let tool = GlobTool;
    let input = json!({
        "pattern": "*.rs",
        "path": file_path.to_str().unwrap()
    });
    let ctx = make_ctx(&dir);
    let cancel = CancellationToken::new();

    let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
    assert!(result.is_error, "Expected error when path points to a file, not a directory");
}
