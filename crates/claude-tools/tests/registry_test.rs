use async_trait::async_trait;
use claude_core::types::events::ToolResultData;
use claude_tools::registry::*;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

struct MockTool;
struct OtherTool;

#[async_trait]
impl ToolExecutor for MockTool {
    fn name(&self) -> &str {
        "MockTool"
    }
    fn aliases(&self) -> &[&str] {
        &["mock", "mt"]
    }
    fn input_schema(&self) -> Value {
        json!({"type": "object", "properties": {"x": {"type": "string"}}})
    }
    async fn call(
        &self,
        input: &Value,
        _ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> anyhow::Result<ToolResultData> {
        Ok(ToolResultData {
            data: json!({"echo": input}),
            is_error: false,
        })
    }
}

#[async_trait]
impl ToolExecutor for OtherTool {
    fn name(&self) -> &str {
        "OtherTool"
    }
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    async fn call(
        &self,
        _input: &Value,
        _ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> anyhow::Result<ToolResultData> {
        Ok(ToolResultData {
            data: json!("ok"),
            is_error: false,
        })
    }
}

#[test]
fn test_register_and_get_by_name() {
    let mut reg = ToolRegistry::new();
    reg.register(Arc::new(MockTool));
    assert!(reg.get("MockTool").is_some());
}

#[test]
fn test_get_by_alias() {
    let mut reg = ToolRegistry::new();
    reg.register(Arc::new(MockTool));
    assert!(reg.get("mock").is_some());
    assert!(reg.get("mt").is_some());
}

#[test]
fn test_get_unknown_returns_none() {
    let reg = ToolRegistry::new();
    assert!(reg.get("NonExistent").is_none());
}

#[test]
fn test_all_returns_registered_tools() {
    let mut reg = ToolRegistry::new();
    reg.register(Arc::new(MockTool));
    assert_eq!(reg.all().len(), 1);
}

#[test]
fn test_all_preserves_registration_order() {
    let mut reg = ToolRegistry::new();
    reg.register(Arc::new(MockTool));
    reg.register(Arc::new(OtherTool));

    let names: Vec<String> = reg
        .all()
        .iter()
        .map(|tool| tool.name().to_string())
        .collect();
    assert_eq!(names, vec!["MockTool", "OtherTool"]);
}

#[test]
fn test_remove_updates_registration_order() {
    let mut reg = ToolRegistry::new();
    reg.register(Arc::new(MockTool));
    reg.register(Arc::new(OtherTool));

    assert!(reg.remove("MockTool").is_some());
    let names: Vec<String> = reg
        .all()
        .iter()
        .map(|tool| tool.name().to_string())
        .collect();
    assert_eq!(names, vec!["OtherTool"]);
}
