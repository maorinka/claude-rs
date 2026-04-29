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

fn matches(result: &claude_core::types::events::ToolResultData) -> Vec<String> {
    result.data["matches"]
        .as_array()
        .expect("matches array")
        .iter()
        .filter_map(|value| value.as_str().map(str::to_string))
        .collect()
}

#[tokio::test]
async fn test_exact_search_bash_finds_loaded_bash() {
    let result = search("bash").await;
    assert!(!result.is_error);
    assert_eq!(matches(&result), vec!["Bash"]);
    assert_eq!(result.data["query"], "bash");
}

#[tokio::test]
async fn test_keyword_search_only_searches_deferred_tools() {
    let tool = ToolSearchTool::new(vec![(
        "Read".to_string(),
        "Read a local file from disk".to_string(),
    )]);
    let result = tool
        .call(
            &json!({ "query": "file" }),
            &make_ctx(),
            CancellationToken::new(),
            None,
        )
        .await
        .expect("call should not fail");
    assert!(!result.is_error);
    assert!(matches(&result).is_empty());
    assert_eq!(result.data["total_deferred_tools"], 0);
}

#[tokio::test]
async fn test_search_max_results_respected() {
    let tool = ToolSearchTool::new(vec![
        ("mcp__alpha__create".to_string(), "create item".to_string()),
        ("mcp__alpha__delete".to_string(), "delete item".to_string()),
        ("mcp__alpha__list".to_string(), "list items".to_string()),
    ]);
    let result = tool
        .call(
            &json!({ "query": "alpha", "max_results": 2 }),
            &make_ctx(),
            CancellationToken::new(),
            None,
        )
        .await
        .expect("call should not fail");

    assert!(!result.is_error);
    let names = matches(&result);
    assert!(
        names.len() <= 2,
        "should return at most 3 results, got {}",
        names.len()
    );
    assert_eq!(result.data["total_deferred_tools"], 3);
}

#[tokio::test]
async fn test_search_no_results_for_gibberish() {
    let result = search("xyzzy_gibberish_42").await;
    assert!(!result.is_error);
    assert!(
        matches(&result).is_empty(),
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
    assert_eq!(matches(&result), vec!["CustomTwo"]);
    assert_eq!(result.data["query"], "select:customtwo");
    assert_eq!(result.data["total_deferred_tools"], 0);
}

#[tokio::test]
async fn test_search_uses_deferred_mcp_snapshot_and_camel_case() {
    let tool = ToolSearchTool::new(vec![(
        "mcp__custom_mcp__fetcher".to_string(),
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
    assert_eq!(matches(&result), vec!["mcp__custom_mcp__fetcher"]);
    assert_eq!(result.data["total_deferred_tools"], 1);
}

#[tokio::test]
async fn test_search_uses_ts_should_defer_builtin_metadata() {
    let tool = ToolSearchTool::new(vec![
        (
            "TodoWrite".to_string(),
            "Update and maintain the session todo list".to_string(),
        ),
        ("Read".to_string(), "Read a local file".to_string()),
    ]);
    let result = tool
        .call(
            &json!({ "query": "todo" }),
            &make_ctx(),
            CancellationToken::new(),
            None,
        )
        .await
        .expect("call should not fail");

    assert!(!result.is_error);
    assert_eq!(matches(&result), vec!["TodoWrite"]);
    assert_eq!(result.data["total_deferred_tools"], 1);
}

#[tokio::test]
async fn test_required_only_query_scores_required_terms_like_ts() {
    let tool = ToolSearchTool::new(vec![(
        "mcp__slack__send_message".to_string(),
        "Send a message to Slack".to_string(),
    )]);
    let result = tool
        .call(
            &json!({ "query": "+slack" }),
            &make_ctx(),
            CancellationToken::new(),
            None,
        )
        .await
        .expect("call should not fail");

    assert!(!result.is_error);
    assert_eq!(matches(&result), vec!["mcp__slack__send_message"]);
    assert_eq!(result.data["total_deferred_tools"], 1);
}

#[tokio::test]
async fn test_search_result_matches_ts_output_contract() {
    let result = search("glob").await;
    assert!(!result.is_error);
    assert!(result.data.get("matches").is_some());
    assert!(result.data.get("query").is_some());
    assert!(result.data.get("total_deferred_tools").is_some());
    assert!(result.data.get("tools").is_none());
}
