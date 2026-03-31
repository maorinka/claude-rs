use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

/// AgentTool is a stub for spawning sub-agents.
/// Full implementation requires server-side orchestration support.
pub struct AgentTool;

#[async_trait]
impl ToolExecutor for AgentTool {
    fn name(&self) -> &str {
        "Agent"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "The prompt or task for the sub-agent to execute"
                },
                "description": {
                    "type": "string",
                    "description": "Optional human-readable description of the sub-agent's purpose"
                },
                "model": {
                    "type": "string",
                    "description": "Optional model to use for the sub-agent"
                }
            },
            "required": ["prompt"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }

    fn is_read_only(&self, _input: &Value) -> bool {
        false
    }

    async fn call(
        &self,
        input: &Value,
        _ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let prompt = match input["prompt"].as_str() {
            Some(p) => p.to_string(),
            None => {
                return Ok(ToolResultData {
                    data: json!({ "error": "missing required parameter: prompt" }),
                    is_error: true,
                });
            }
        };

        // Stub: subagent spawning not yet implemented
        Ok(ToolResultData {
            data: json!({
                "status": "completed",
                "prompt": prompt,
                "result": "Sub-agent spawning is not yet fully implemented in this environment. \
                           This tool requires server-side orchestration support to spawn and \
                           coordinate sub-agents."
            }),
            is_error: false,
        })
    }
}
