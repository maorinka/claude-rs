use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

pub struct EnterPlanModeTool;

#[async_trait]
impl ToolExecutor for EnterPlanModeTool {
    fn name(&self) -> &str {
        "EnterPlanMode"
    }

    fn description(&self) -> String {
        "Switch to plan mode for describing actions before executing them. In plan mode, tools are not actually executed.".to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    fn is_read_only(&self, _input: &Value) -> bool {
        true
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    async fn call(
        &self,
        _input: &Value,
        _ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        Ok(ToolResultData {
            data: json!({
                "mode": "plan",
                "message": "Plan mode is now active. Describe your plan before taking any actions."
            }),
            is_error: false,
        })
    }
}

pub struct ExitPlanModeTool;

#[async_trait]
impl ToolExecutor for ExitPlanModeTool {
    fn name(&self) -> &str {
        "ExitPlanMode"
    }

    fn description(&self) -> String {
        "Exit plan mode and return to normal execution mode where tools are actually executed.".to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    fn is_read_only(&self, _input: &Value) -> bool {
        true
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    async fn call(
        &self,
        _input: &Value,
        _ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        Ok(ToolResultData {
            data: json!({
                "mode": "normal",
                "message": "Normal mode is now active. You may proceed with actions."
            }),
            is_error: false,
        })
    }
}
