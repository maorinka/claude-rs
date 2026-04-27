use claude_tools::registry::{ToolExecutor, ToolUseContext};
use claude_tools::tool_search::{
    get_tool_search_mode, is_tool_search_enabled_optimistic, ToolSearchMode, ToolSearchTool,
};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Mutex;
use tokio_util::sync::CancellationToken;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn clear_tool_search_env() {
    for key in [
        "CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS",
        "ENABLE_TOOL_SEARCH",
        "ANTHROPIC_BASE_URL",
        "CLAUDE_CODE_USE_BEDROCK",
        "CLAUDE_CODE_USE_VERTEX",
        "CLAUDE_CODE_USE_FOUNDRY",
        "USER_TYPE",
    ] {
        std::env::remove_var(key);
    }
}

fn make_ctx() -> ToolUseContext {
    ToolUseContext::for_test(
        PathBuf::from("/tmp"),
        std::sync::Arc::new(std::sync::Mutex::new(
            claude_tools::registry::ReadFileState::new(),
        )),
        claude_tools::registry::PermissionMode::Default,
    )
}

async fn search(query: &str) -> claude_core::types::events::ToolResultData {
    let tool = ToolSearchTool::default();
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
    assert!(
        !tools.is_empty(),
        "should find at least one tool matching 'bash'"
    );
    let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    assert!(names.contains(&"Bash"), "Bash tool should be in results");
}

#[tokio::test]
async fn test_search_file_finds_read_write() {
    let result = search("file").await;
    assert!(!result.is_error);
    let tools = result.data["tools"].as_array().expect("tools array");
    let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    // "Read", "Write", "Edit" all mention "file" in their descriptions
    let has_read_or_write = names
        .iter()
        .any(|n| *n == "Read" || *n == "Write" || *n == "Edit");
    assert!(
        has_read_or_write,
        "should find file-related tools, got: {:?}",
        names
    );
}

#[tokio::test]
async fn test_search_max_results_respected() {
    let tool = ToolSearchTool::default();
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
    assert!(
        tools.len() <= 3,
        "should return at most 3 results, got {}",
        tools.len()
    );
}

#[tokio::test]
async fn test_search_no_results_for_gibberish() {
    let result = search("xyzzy_gibberish_42").await;
    assert!(!result.is_error);
    let tools = result.data["tools"].as_array().expect("tools array");
    assert!(
        tools.is_empty(),
        "should return no results for nonsense query"
    );
}

#[tokio::test]
async fn test_search_missing_query() {
    let tool = ToolSearchTool::default();
    let result = tool
        .call(&json!({}), &make_ctx(), CancellationToken::new(), None)
        .await
        .expect("call should not fail");
    assert!(result.is_error, "missing query should produce an error");
}

#[test]
fn test_tool_search_is_read_only_and_concurrency_safe() {
    let tool = ToolSearchTool::default();
    let input = json!({ "query": "bash" });
    assert!(tool.is_read_only(&input));
    assert!(tool.is_concurrency_safe(&input));
}

#[test]
fn test_tool_search_mode_matches_ts_env_mapping() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    clear_tool_search_env();
    assert_eq!(get_tool_search_mode(), ToolSearchMode::ToolSearch);

    std::env::set_var("ENABLE_TOOL_SEARCH", "auto");
    assert_eq!(get_tool_search_mode(), ToolSearchMode::ToolSearchAuto);

    std::env::set_var("ENABLE_TOOL_SEARCH", "auto:0");
    assert_eq!(get_tool_search_mode(), ToolSearchMode::ToolSearch);

    std::env::set_var("ENABLE_TOOL_SEARCH", "auto:100");
    assert_eq!(get_tool_search_mode(), ToolSearchMode::Standard);

    std::env::set_var("ENABLE_TOOL_SEARCH", "false");
    assert_eq!(get_tool_search_mode(), ToolSearchMode::Standard);

    std::env::set_var("ENABLE_TOOL_SEARCH", "true");
    std::env::set_var("CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS", "1");
    assert_eq!(get_tool_search_mode(), ToolSearchMode::Standard);

    clear_tool_search_env();
}

#[test]
fn test_tool_search_optimistic_disables_default_on_non_first_party_base_url() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    clear_tool_search_env();
    std::env::set_var("ANTHROPIC_BASE_URL", "http://127.0.0.1:8787");
    assert!(!is_tool_search_enabled_optimistic());

    std::env::set_var("ENABLE_TOOL_SEARCH", "true");
    assert!(is_tool_search_enabled_optimistic());

    clear_tool_search_env();
}

#[tokio::test]
async fn test_select_query_matches_exact_names() {
    let tool = ToolSearchTool::new(vec![
        ("CustomOne".to_string(), "First dynamic tool".to_string()),
        ("CustomTwo".to_string(), "Second dynamic tool".to_string()),
    ]);
    let result = tool
        .call(
            &json!({ "query": "select:customtwo" }),
            &make_ctx(),
            CancellationToken::new(),
            None,
        )
        .await
        .expect("call should not fail");

    assert!(!result.is_error);
    let tools = result.data["tools"].as_array().expect("tools array");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["name"], "CustomTwo");
}

#[tokio::test]
async fn test_search_uses_dynamic_snapshot_and_camel_case() {
    let tool = ToolSearchTool::new(vec![(
        "CustomMcpFetcher".to_string(),
        "Fetches resources from a runtime server".to_string(),
    )]);
    let result = tool
        .call(
            &json!({ "query": "mcp fetcher" }),
            &make_ctx(),
            CancellationToken::new(),
            None,
        )
        .await
        .expect("call should not fail");

    assert!(!result.is_error);
    let tools = result.data["tools"].as_array().expect("tools array");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["name"], "CustomMcpFetcher");
}

#[tokio::test]
async fn test_search_results_have_name_and_description() {
    let result = search("glob").await;
    assert!(!result.is_error);
    let tools = result.data["tools"].as_array().expect("tools array");
    for tool in tools {
        assert!(tool.get("name").is_some(), "each result should have a name");
        assert!(
            tool.get("description").is_some(),
            "each result should have a description"
        );
    }
}
