use claude_tools::notebook_edit::NotebookEditTool;
use claude_tools::registry::{ToolExecutor, ToolUseContext};
use serde_json::json;
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;

fn make_ctx() -> ToolUseContext {
    ToolUseContext {
        working_directory: PathBuf::from("/tmp"),
        read_file_state: std::sync::Arc::new(std::sync::Mutex::new(
            claude_tools::registry::ReadFileState::new(),
        )),
    }
}

/// Creates a minimal .ipynb notebook with two cells and returns the path.
async fn write_sample_notebook() -> tempfile::NamedTempFile {
    let notebook = json!({
        "nbformat": 4,
        "nbformat_minor": 5,
        "metadata": {
            "kernelspec": {
                "display_name": "Python 3",
                "language": "python",
                "name": "python3"
            }
        },
        "cells": [
            {
                "cell_type": "code",
                "source": "print('hello')",
                "metadata": {},
                "outputs": [],
                "execution_count": null
            },
            {
                "cell_type": "markdown",
                "source": "# Title\nSome text here.",
                "metadata": {}
            }
        ]
    });

    let f = tempfile::Builder::new()
        .suffix(".ipynb")
        .tempfile()
        .unwrap();
    std::fs::write(f.path(), serde_json::to_string_pretty(&notebook).unwrap()).unwrap();
    f
}

#[tokio::test]
async fn test_edit_code_cell_source() {
    let nb_file = write_sample_notebook().await;
    let path = nb_file.path().to_str().unwrap().to_string();

    let tool = NotebookEditTool;
    let input = json!({
        "notebook_path": path,
        "cell_index": 0,
        "new_source": "print('world')"
    });

    let result = tool
        .call(&input, &make_ctx(), CancellationToken::new(), None)
        .await
        .unwrap();

    assert!(!result.is_error, "edit should succeed: {:?}", result.data);
    assert_eq!(result.data["cellIndex"], 0);
    assert_eq!(result.data["cellType"], "code");
    assert_eq!(result.data["previousSource"], "print('hello')");
    assert_eq!(result.data["filePath"], path);

    // Read back and verify
    let updated_raw = tokio::fs::read_to_string(&path).await.unwrap();
    let updated: serde_json::Value = serde_json::from_str(&updated_raw).unwrap();
    let source = updated["cells"][0]["source"].as_str().unwrap();
    assert_eq!(source, "print('world')");
}

#[tokio::test]
async fn test_edit_markdown_cell_source() {
    let nb_file = write_sample_notebook().await;
    let path = nb_file.path().to_str().unwrap().to_string();

    let tool = NotebookEditTool;
    let input = json!({
        "notebook_path": path,
        "cell_index": 1,
        "new_source": "# New Title\nUpdated content."
    });

    let result = tool
        .call(&input, &make_ctx(), CancellationToken::new(), None)
        .await
        .unwrap();

    assert!(!result.is_error);
    assert_eq!(result.data["cellType"], "markdown");
    assert!(
        result.data["previousSource"].as_str().unwrap().contains("Title"),
        "previousSource should contain original markdown"
    );

    // Verify written content
    let updated_raw = tokio::fs::read_to_string(&path).await.unwrap();
    let updated: serde_json::Value = serde_json::from_str(&updated_raw).unwrap();
    assert_eq!(
        updated["cells"][1]["source"].as_str().unwrap(),
        "# New Title\nUpdated content."
    );
}

#[tokio::test]
async fn test_edit_cell_type_change() {
    let nb_file = write_sample_notebook().await;
    let path = nb_file.path().to_str().unwrap().to_string();

    let tool = NotebookEditTool;
    let input = json!({
        "notebook_path": path,
        "cell_index": 0,
        "new_source": "# Now markdown",
        "cell_type": "markdown"
    });

    let result = tool
        .call(&input, &make_ctx(), CancellationToken::new(), None)
        .await
        .unwrap();

    assert!(!result.is_error);
    assert_eq!(result.data["cellType"], "markdown");

    let updated_raw = tokio::fs::read_to_string(&path).await.unwrap();
    let updated: serde_json::Value = serde_json::from_str(&updated_raw).unwrap();
    assert_eq!(updated["cells"][0]["cell_type"].as_str().unwrap(), "markdown");
}

#[tokio::test]
async fn test_edit_out_of_bounds_cell_returns_error() {
    let nb_file = write_sample_notebook().await;
    let path = nb_file.path().to_str().unwrap().to_string();

    let tool = NotebookEditTool;
    let input = json!({
        "notebook_path": path,
        "cell_index": 99,
        "new_source": "x = 1"
    });

    let result = tool
        .call(&input, &make_ctx(), CancellationToken::new(), None)
        .await
        .unwrap();

    assert!(result.is_error, "out-of-bounds index should return error");
    let err = result.data["error"].as_str().unwrap_or("");
    assert!(
        err.contains("out of bounds") || err.contains("99"),
        "error should mention index issue: {}",
        err
    );
}

#[tokio::test]
async fn test_edit_nonexistent_notebook_returns_error() {
    let tool = NotebookEditTool;
    let input = json!({
        "notebook_path": "/tmp/this_notebook_does_not_exist_xyz.ipynb",
        "cell_index": 0,
        "new_source": "x = 1"
    });

    let result = tool
        .call(&input, &make_ctx(), CancellationToken::new(), None)
        .await
        .unwrap();

    assert!(result.is_error, "nonexistent notebook should return error");
}

#[test]
fn test_notebook_edit_is_destructive() {
    let tool = NotebookEditTool;
    let input = json!({});
    assert!(tool.is_destructive(&input));
    assert!(!tool.is_concurrency_safe(&input));
    assert!(!tool.is_read_only(&input));
    assert_eq!(tool.name(), "NotebookEdit");
}
