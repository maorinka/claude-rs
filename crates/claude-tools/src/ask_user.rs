use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

pub struct AskUserQuestionTool;

#[async_trait]
impl ToolExecutor for AskUserQuestionTool {
    fn name(&self) -> &str {
        "AskUser"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "The question to ask the user."
                },
                "options": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional list of choices to present to the user."
                }
            },
            "required": ["question"]
        })
    }

    fn is_read_only(&self, _input: &Value) -> bool {
        true
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
        let question = match input["question"].as_str() {
            Some(q) => q,
            None => {
                return Ok(ToolResultData {
                    data: json!({ "error": "missing required field: question" }),
                    is_error: true,
                });
            }
        };

        let options = input.get("options").and_then(|o| o.as_array()).cloned();

        // Stub: actual UI integration happens at the TUI layer.
        let answer = if let Some(opts) = &options {
            format!(
                "Question '{}' was asked with options: {}. Awaiting user response via TUI.",
                question,
                opts.iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        } else {
            format!(
                "Question '{}' was asked. Awaiting user response via TUI.",
                question
            )
        };

        Ok(ToolResultData {
            data: json!({ "answer": answer }),
            is_error: false,
        })
    }
}
