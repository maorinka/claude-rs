use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

pub struct SnipTool;

#[async_trait]
impl ToolExecutor for SnipTool {
    fn name(&self) -> &str {
        "Snip"
    }

    fn description(&self) -> String {
        "Trim conversation history to free up context window space. Marks a range \
         of earlier messages for removal or collapse. The snipped content is no longer \
         sent to the model on subsequent turns but remains in the session transcript. \
         Use this proactively when the context window is getting full and older \
         messages are no longer relevant to the current task."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "reason": { "type": "string", "description": "Why this part of the conversation is being snipped." },
                "messageRange": {
                    "type": "object",
                    "description": "Range of messages to snip.",
                    "properties": {
                        "from": { "type": "integer", "description": "Start message index (0-based, inclusive)." },
                        "to": { "type": "integer", "description": "End message index (0-based, inclusive)." }
                    }
                },
                "keepSummary": { "type": "boolean", "description": "Whether to generate a brief summary of snipped content. Defaults to true." }
            },
            "required": ["reason"]
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
        let reason = match input.get("reason").and_then(|v| v.as_str()) {
            Some(r) => r,
            None => {
                return Ok(ToolResultData {
                    data: json!({ "error": "missing required field: reason" }),
                    is_error: true,
                })
            }
        };
        let from = input
            .get("messageRange")
            .and_then(|r| r.get("from"))
            .and_then(|v| v.as_u64());
        let to = input
            .get("messageRange")
            .and_then(|r| r.get("to"))
            .and_then(|v| v.as_u64());
        let keep_summary = input
            .get("keepSummary")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        Ok(ToolResultData {
            data: json!({
                "snipRequested": true,
                "reason": reason,
                "messageRange": { "from": from, "to": to },
                "keepSummary": keep_summary,
                "message": format!("Snip requested: {}. The engine will trim the specified messages from context.", reason),
            }),
            is_error: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ReadFileState;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn make_ctx() -> ToolUseContext {
        ToolUseContext::for_test(PathBuf::from("/tmp"), Arc::new(std::sync::Mutex::new(ReadFileState::new())), crate::registry::PermissionMode::Default)
    }

    #[tokio::test]
    async fn snip_with_reason() {
        let tool = SnipTool;
        let result = tool
            .call(
                &json!({ "reason": "Earlier debugging is no longer relevant" }),
                &make_ctx(),
                CancellationToken::new(),
                None,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.data["snipRequested"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn snip_missing_reason() {
        let tool = SnipTool;
        let result = tool
            .call(&json!({}), &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.data["error"].as_str().unwrap().contains("reason"));
    }

    #[tokio::test]
    async fn snip_tool_properties() {
        let tool = SnipTool;
        assert_eq!(tool.name(), "Snip");
        assert!(!tool.is_read_only(&json!({})));
    }
}
