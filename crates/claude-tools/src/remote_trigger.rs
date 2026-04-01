use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::remote::client::RemoteClient;
use claude_core::remote::types::RemoteTaskConfig;
use claude_core::types::events::ToolResultData;

/// RemoteTriggerTool dispatches a task to Anthropic's cloud execution environment.
///
/// Input:  { prompt, model?, max_turns?, auth_token? }
/// Output: { task_id, status, session_url? }
///
/// The tool calls RemoteClient::create_task() to submit the prompt for cloud execution.
/// An auth_token must be supplied either in the input JSON or via the ANTHROPIC_API_KEY
/// environment variable.
pub struct RemoteTriggerTool;

#[async_trait]
impl ToolExecutor for RemoteTriggerTool {
    fn name(&self) -> &str {
        "RemoteTrigger"
    }

    fn description(&self) -> String {
        "Dispatch a prompt to Anthropic's cloud execution environment as a remote task. \
         Returns the task ID, initial status, and an optional session URL for monitoring."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["prompt"],
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "The prompt/instructions to execute remotely"
                },
                "model": {
                    "type": "string",
                    "description": "Optional model to use for remote execution (e.g. claude-opus-4-5)"
                },
                "max_turns": {
                    "type": "integer",
                    "description": "Maximum number of turns for the remote session",
                    "minimum": 1
                },
                "working_directory": {
                    "type": "string",
                    "description": "Remote working directory for the task"
                },
                "auth_token": {
                    "type": "string",
                    "description": "Anthropic API token. Falls back to ANTHROPIC_API_KEY env var."
                }
            }
        })
    }

    async fn call(
        &self,
        input: &Value,
        _ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let prompt = match input["prompt"].as_str() {
            Some(p) if !p.trim().is_empty() => p.to_string(),
            _ => {
                return Ok(ToolResultData {
                    data: json!({ "error": "prompt is required and must be non-empty" }),
                    is_error: true,
                });
            }
        };

        // Resolve auth token: input field > ANTHROPIC_API_KEY env var
        let auth_token = input
            .get("auth_token")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
            .unwrap_or_default();

        if auth_token.is_empty() {
            return Ok(ToolResultData {
                data: json!({
                    "error": "auth_token is required. Pass it via the auth_token field or set ANTHROPIC_API_KEY."
                }),
                is_error: true,
            });
        }

        let model = input
            .get("model")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let max_turns = input
            .get("max_turns")
            .and_then(|v| v.as_u64())
            .map(|n| n as u32);

        let working_directory = input
            .get("working_directory")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let config = RemoteTaskConfig {
            prompt,
            model,
            max_turns,
            working_directory,
        };

        let client = RemoteClient::new(&auth_token);

        match client.create_task(config).await {
            Ok(status) => {
                let session_url = status.session_url.clone();
                Ok(ToolResultData {
                    data: json!({
                        "task_id": status.task_id,
                        "status": serde_json::to_value(&status.status).unwrap_or(json!("unknown")),
                        "session_url": session_url,
                    }),
                    is_error: false,
                })
            }
            Err(e) => Ok(ToolResultData {
                data: json!({ "error": format!("Remote task creation failed: {}", e) }),
                is_error: true,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_remote_trigger_tool_name() {
        let tool = RemoteTriggerTool;
        assert_eq!(tool.name(), "RemoteTrigger");
    }

    #[test]
    fn test_remote_trigger_tool_schema() {
        let tool = RemoteTriggerTool;
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "prompt"));
        assert!(schema["properties"]["prompt"].is_object());
        assert!(schema["properties"]["model"].is_object());
        assert!(schema["properties"]["max_turns"].is_object());
        assert!(schema["properties"]["auth_token"].is_object());
    }

    #[test]
    fn test_remote_trigger_tool_description() {
        let tool = RemoteTriggerTool;
        let desc = tool.description();
        assert!(desc.contains("remote"));
    }
}
