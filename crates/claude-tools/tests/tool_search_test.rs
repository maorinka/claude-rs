use claude_tools::tool_search::ToolSearchTool;
use claude_tools::registry::{ToolExecutor, ToolUseContext};
use serde_json::json;
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;

fn make_ctx() -> ToolUseContext {
    ToolUseContext {
        working_directory: PathBuf::from("/tmp"),
    }
}

async fn search(query: &str) -> claude_core::types::events::ToolResultData {
    let tool = ToolSearchTool;
    tool.call(
        &json!({ "query": query }),
        &make_ctx(),
        CancellationToken::new(),
        None,
    )
    .await
    .expect("call should not fail")
}

#[tokio::test]
async fn test_search_bash_finds_bash() {
    let result = search("bash").await;
    assert!(!result.is_error);
    let tools = result.data["tools"].as_array().expect("tools array");
    assert!(!tools.is_empty(), "should find at least one tool matching 'bash'");
    let names: Vec<&str> = tools
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert!(names.contains(&"Bash"), "Bash tool should be in results");
}

#[tokio::test]
async fn test_search_file_finds_read_write() {
    let result = search("file").await;
    assert!(!result.is_error);
    let tools = result.data["tools"].as_array().expect("tools array");
    let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    // "Read", "Write", "Edit" all mention "file" in their descriptions
    let has_read_or_write = names.iter().any(|n| *n == "Read" || *n == "Write" || *n == "Edit");
    assert!(has_read_or_write, "should find file-related tools, got: {:?}", names);
}

#[tokio::test]
async fn test_search_max_results_respected() {
    let tool = ToolSearchTool;
    // "a" is likely to match many tools
    let result = tool
        .call(
            &json!({ "query": "a", "max_results": 3 }),
            &make_ctx(),
            CancellationToken::new(),
            None,
        )
        .await
        .expect("call should not fail");

    assert!(!result.is_error);
    let tools = result.data["tools"].as_array().expect("tools array");
    assert!(tools.len() <= 3, "should return at most 3 results, got {}", tools.len());
}

#[tokio::test]
async fn test_search_no_results_for_gibberish() {
    let result = search("xyzzy_gibberish_42").await;
    assert!(!result.is_error);
    let tools = result.data["tools"].as_array().expect("tools array");
    assert!(tools.is_empty(), "should return no results for nonsense query");
}

#[tokio::test]
async fn test_search_missing_query() {
    let tool = ToolSearchTool;
    let result = tool
        .call(&json!({}), &make_ctx(), CancellationToken::new(), None)
        .await
        .expect("call should not fail");
    assert!(result.is_error, "missing query should produce an error");
}

#[test]
fn test_tool_search_is_read_only_and_concurrency_safe() {
    let tool = ToolSearchTool;
    let input = json!({ "query": "bash" });
    assert!(tool.is_read_only(&input));
    assert!(tool.is_concurrency_safe(&input));
}

#[tokio::test]
async fn test_search_results_have_name_and_description() {
    let result = search("glob").await;
    assert!(!result.is_error);
    let tools = result.data["tools"].as_array().expect("tools array");
    for tool in tools {
        assert!(tool.get("name").is_some(), "each result should have a name");
        assert!(tool.get("description").is_some(), "each result should have a description");
    }
}
