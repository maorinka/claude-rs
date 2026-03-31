use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

pub struct BriefTool;

#[async_trait]
impl ToolExecutor for BriefTool {
    fn name(&self) -> &str {
        "Brief"
    }

    fn description(&self) -> String {
        "Toggle brief mode for more concise responses with less explanation and commentary.".to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "enabled": {
                    "type": "boolean",
                    "description": "Whether to enable brief mode."
                }
            },
            "required": ["enabled"]
        })
    }

    fn is_read_only(&self, _input: &Value) -> bool {
        false
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }

    async fn call(
        &self,
        input: &Value,
        _ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let enabled = match input["enabled"].as_bool() {
            Some(b) => b,
            None => {
                return Ok(ToolResultData {
                    data: json!({ "error": "missing required field: enabled (must be boolean)" }),
                    is_error: true,
                });
            }
        };

        Ok(ToolResultData {
            data: json!({ "brief_mode": enabled }),
            is_error: false,
        })
    }
}
