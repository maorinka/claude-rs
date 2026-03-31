use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

pub struct SendMessageTool;

#[async_trait]
impl ToolExecutor for SendMessageTool {
    fn name(&self) -> &str {
        "SendMessage"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "to": {
                    "type": "string",
                    "description": "The recipient agent or channel to send the message to."
                },
                "content": {
                    "type": "string",
                    "description": "The message content to send."
                }
            },
            "required": ["to", "content"]
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
        let to = match input["to"].as_str() {
            Some(t) => t,
            None => {
                return Ok(ToolResultData {
                    data: json!({ "error": "missing required field: to" }),
                    is_error: true,
                });
            }
        };

        if input["content"].as_str().is_none() {
            return Ok(ToolResultData {
                data: json!({ "error": "missing required field: content" }),
                is_error: true,
            });
        }

        // Stub: inter-agent messaging is not yet connected.
        Ok(ToolResultData {
            data: json!({
                "sent": true,
                "to": to
            }),
            is_error: false,
        })
    }
}
