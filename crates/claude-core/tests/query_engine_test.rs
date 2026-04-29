use claude_core::api::client::{ApiClient, ApiConfig, AuthMethod};
use claude_core::query::engine::*;
use claude_core::query::state::*;
use claude_core::types::message::StopReason;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[test]
fn test_query_state_variants() {
    let s = QueryState::Querying;
    assert!(matches!(s, QueryState::Querying));

    let s = QueryState::Terminal {
        stop_reason: StopReason::EndTurn,
        transition: TransitionReason::Completed,
    };
    assert!(matches!(s, QueryState::Terminal { .. }));
}

#[test]
fn test_transition_reason_variants() {
    let t = TransitionReason::Completed;
    assert!(matches!(t, TransitionReason::Completed));

    let t = TransitionReason::MaxTurns;
    assert!(matches!(t, TransitionReason::MaxTurns));
}

#[test]
fn test_query_engine_new() {
    let config = ApiConfig::default();
    let auth = AuthMethod::ApiKey("test".into());
    let client = ApiClient::new(config, auth);
    let cancel = CancellationToken::new();
    let engine = QueryEngine::new(client, vec![], vec![], cancel);
    assert!(matches!(engine.state(), QueryState::Querying));
    assert!(engine.messages().is_empty());
}

#[test]
fn test_query_engine_add_messages() {
    let config = ApiConfig::default();
    let auth = AuthMethod::ApiKey("test".into());
    let client = ApiClient::new(config, auth);
    let cancel = CancellationToken::new();
    let mut engine = QueryEngine::new(client, vec![], vec![], cancel);

    engine.add_user_message("hello");
    assert_eq!(engine.messages().len(), 1);
    assert_eq!(engine.messages()[0]["role"], "user");

    engine.add_tool_result("tu_1", "result data", false);
    assert_eq!(engine.messages().len(), 2);
    assert_eq!(engine.messages()[1]["content"][0]["content"], "result data");
    assert!(engine.messages()[1]["content"][0].get("is_error").is_none());
}

#[test]
fn test_query_engine_batches_parallel_tool_results() {
    let config = ApiConfig::default();
    let auth = AuthMethod::ApiKey("test".into());
    let client = ApiClient::new(config, auth);
    let cancel = CancellationToken::new();
    let mut engine = QueryEngine::new(client, vec![], vec![], cancel);

    engine.add_user_message("hello");
    engine.add_tool_result("tu_1", "first", false);
    engine.add_tool_result("tu_2", "second", false);

    assert_eq!(engine.messages().len(), 2);
    let content = engine.messages()[1]["content"].as_array().unwrap();
    assert_eq!(content.len(), 2);
    assert_eq!(content[0]["tool_use_id"], "tu_1");
    assert_eq!(content[0]["content"], "first");
    assert_eq!(content[1]["tool_use_id"], "tu_2");
    assert_eq!(content[1]["content"], "second");
}

#[test]
fn test_query_engine_can_preserve_false_is_error() {
    let config = ApiConfig::default();
    let auth = AuthMethod::ApiKey("test".into());
    let client = ApiClient::new(config, auth);
    let cancel = CancellationToken::new();
    let mut engine = QueryEngine::new(client, vec![], vec![], cancel);

    engine.add_tool_result_with_error_field("tu_1", "ok", false, true);
    assert_eq!(engine.messages()[0]["content"][0]["is_error"], false);
}

#[tokio::test]
async fn test_query_engine_cancel_before_run() {
    let config = ApiConfig::default();
    let auth = AuthMethod::ApiKey("test".into());
    let client = ApiClient::new(config, auth);
    let cancel = CancellationToken::new();
    cancel.cancel();

    let mut engine = QueryEngine::new(client, vec![], vec![], cancel);
    engine.add_user_message("hello");

    let (tx, mut _rx) = mpsc::channel(100);
    let result = engine.run_turn(&tx).await.unwrap();
    assert!(matches!(result, TurnResult::Done(StopReason::EndTurn)));
    assert!(matches!(engine.state(), QueryState::Terminal { .. }));
}

#[test]
fn test_tool_use_info() {
    let info = ToolUseInfo {
        id: "tu_1".into(),
        name: "Bash".into(),
        input: serde_json::json!({"command": "ls"}),
        message_id: Some("msg_1".into()),
        model: Some("claude-sonnet-4-6".into()),
        usage: None,
    };
    assert_eq!(info.name, "Bash");
    assert_eq!(info.input["command"], "ls");
}
