use claude_tools::config_tool::ConfigTool;
use claude_tools::registry::{ToolExecutor, ToolUseContext};
use serde_json::{json, Value};
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

/// Build a `ConfigTool` that reads/writes a temp file unique to each test.
fn make_tool_with_temp_settings() -> (ConfigTool, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let settings_path = dir.path().join("settings.json");
    let tool = ConfigTool {
        settings_path_override: Some(settings_path),
    };
    (tool, dir)
}

async fn call(tool: &ConfigTool, input: Value) -> claude_core::types::events::ToolResultData {
    tool.call(&input, &make_ctx(), CancellationToken::new(), None)
        .await
        .expect("call should not return Err")
}

#[tokio::test]
async fn test_config_list_empty() {
    let (tool, _dir) = make_tool_with_temp_settings();
    let result = call(&tool, json!({ "action": "list" })).await;

    assert!(!result.is_error, "list should succeed even with no settings file");
    assert_eq!(result.data["action"], "list");
    let settings = &result.data["settings"];
    assert!(settings.is_object(), "settings should be a JSON object");
    assert_eq!(settings.as_object().unwrap().len(), 0, "empty settings file");
}

#[tokio::test]
async fn test_config_set_and_get() {
    let (tool, _dir) = make_tool_with_temp_settings();

    // Set a key
    let set_result = call(&tool, json!({
        "action": "set",
        "key": "theme",
        "value": "dark"
    })).await;
    assert!(!set_result.is_error, "set should succeed");
    assert_eq!(set_result.data["action"], "set");
    assert_eq!(set_result.data["key"], "theme");
    assert_eq!(set_result.data["value"], "dark");

    // Get the key back
    let get_result = call(&tool, json!({
        "action": "get",
        "key": "theme"
    })).await;

    assert!(!get_result.is_error, "get should succeed");
    assert_eq!(get_result.data["action"], "get");
    assert_eq!(get_result.data["key"], "theme");
    assert_eq!(get_result.data["value"], "dark");
}

#[tokio::test]
async fn test_config_set_then_list() {
    let (tool, _dir) = make_tool_with_temp_settings();

    call(&tool, json!({ "action": "set", "key": "color", "value": "blue" })).await;
    let list_result = call(&tool, json!({ "action": "list" })).await;

    assert!(!list_result.is_error);
    let settings = &list_result.data["settings"];
    assert_eq!(settings["color"], "blue");
}

#[tokio::test]
async fn test_config_get_missing_key() {
    let (tool, _dir) = make_tool_with_temp_settings();
    let result = call(&tool, json!({ "action": "get", "key": "nonexistent_key" })).await;

    assert!(!result.is_error);
    // Value should be null for a key that doesn't exist
    assert_eq!(result.data["value"], Value::Null);
}

#[tokio::test]
async fn test_config_multiple_set_then_list() {
    let (tool, _dir) = make_tool_with_temp_settings();

    call(&tool, json!({ "action": "set", "key": "a", "value": "1" })).await;
    call(&tool, json!({ "action": "set", "key": "b", "value": "2" })).await;

    let list_result = call(&tool, json!({ "action": "list" })).await;
    assert!(!list_result.is_error);
    let settings = &list_result.data["settings"];
    assert_eq!(settings["a"], "1");
    assert_eq!(settings["b"], "2");
}

#[tokio::test]
async fn test_config_missing_action() {
    let (tool, _dir) = make_tool_with_temp_settings();
    let result = call(&tool, json!({})).await;
    assert!(result.is_error, "missing action should produce an error");
}

#[tokio::test]
async fn test_config_set_missing_key() {
    let (tool, _dir) = make_tool_with_temp_settings();
    let result = call(&tool, json!({ "action": "set", "value": "something" })).await;
    assert!(result.is_error, "set without key should produce an error");
}

#[tokio::test]
async fn test_config_set_missing_value() {
    let (tool, _dir) = make_tool_with_temp_settings();
    let result = call(&tool, json!({ "action": "set", "key": "mykey" })).await;
    assert!(result.is_error, "set without value should produce an error");
}

#[test]
fn test_config_is_read_only_for_get_and_list() {
    let tool = ConfigTool::new();
    assert!(tool.is_read_only(&json!({ "action": "get", "key": "x" })));
    assert!(tool.is_read_only(&json!({ "action": "list" })));
    assert!(!tool.is_read_only(&json!({ "action": "set", "key": "x", "value": "v" })));
}

#[test]
fn test_config_is_destructive_for_set() {
    let tool = ConfigTool::new();
    assert!(tool.is_destructive(&json!({ "action": "set", "key": "x", "value": "v" })));
    assert!(!tool.is_destructive(&json!({ "action": "get", "key": "x" })));
    assert!(!tool.is_destructive(&json!({ "action": "list" })));
}
