use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

/// Maximum sleep duration: 10 minutes (600 seconds).
const MAX_SLEEP_SECONDS: u64 = 600;

pub struct SleepTool;

#[async_trait]
impl ToolExecutor for SleepTool {
    fn name(&self) -> &str {
        "Sleep"
    }

    fn description(&self) -> String {
        r#"Wait for a specified duration. The user can interrupt the sleep at any time.

Use this when the user tells you to sleep or rest, when you have nothing to do, or when you're waiting for something.

You can call this concurrently with other tools -- it won't interfere with them.

Prefer this over `Bash(sleep ...)` -- it doesn't hold a shell process.

Each wake-up costs an API call, but the prompt cache expires after 5 minutes of inactivity -- balance accordingly."#
            .to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "seconds": {
                    "type": "integer",
                    "description": "Number of seconds to sleep (max 600)."
                }
            },
            "required": ["seconds"]
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
        input: &Value,
        _ctx: &ToolUseContext,
        cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let seconds = match input.get("seconds").and_then(|v| v.as_u64()) {
            Some(s) => s,
            None => {
                return Ok(ToolResultData {
                    data: json!({ "error": "missing required field: seconds (must be a positive integer)" }),
                    is_error: true,
                });
            }
        };

        if seconds == 0 {
            return Ok(ToolResultData {
                data: json!({ "slept": 0, "message": "No sleep needed (0 seconds)." }),
                is_error: false,
            });
        }

        let capped = seconds.min(MAX_SLEEP_SECONDS);
        let duration = tokio::time::Duration::from_secs(capped);

        let was_cancelled = tokio::select! {
            _ = tokio::time::sleep(duration) => false,
            _ = cancel.cancelled() => true,
        };

        if was_cancelled {
            Ok(ToolResultData {
                data: json!({
                    "slept": 0,
                    "cancelled": true,
                    "message": "Sleep was interrupted."
                }),
                is_error: false,
            })
        } else {
            Ok(ToolResultData {
                data: json!({
                    "slept": capped,
                    "message": format!("Slept for {} second{}.", capped, if capped == 1 { "" } else { "s" })
                }),
                is_error: false,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ReadFileState;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn make_ctx() -> ToolUseContext {
        ToolUseContext {
            working_directory: PathBuf::from("/tmp"),
            read_file_state: Arc::new(std::sync::Mutex::new(ReadFileState::new())),
        }
    }

    #[tokio::test]
    async fn sleep_zero_seconds() {
        let tool = SleepTool;
        let input = json!({ "seconds": 0 });
        let ctx = make_ctx();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(!result.is_error);
        assert_eq!(result.data["slept"].as_u64().unwrap(), 0);
    }

    #[tokio::test]
    async fn sleep_short_duration() {
        let tool = SleepTool;
        let input = json!({ "seconds": 1 });
        let ctx = make_ctx();
        let cancel = CancellationToken::new();

        let start = std::time::Instant::now();
        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        let elapsed = start.elapsed();

        assert!(!result.is_error);
        assert_eq!(result.data["slept"].as_u64().unwrap(), 1);
        assert!(elapsed >= std::time::Duration::from_millis(900));
    }

    #[tokio::test]
    async fn sleep_cancellation() {
        let tool = SleepTool;
        let input = json!({ "seconds": 60 });
        let ctx = make_ctx();
        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();

        // Cancel after a short delay
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            cancel_clone.cancel();
        });

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(!result.is_error);
        assert!(result.data["cancelled"].as_bool().unwrap_or(false));
    }

    #[tokio::test]
    async fn sleep_missing_seconds() {
        let tool = SleepTool;
        let input = json!({});
        let ctx = make_ctx();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(result.is_error);
        assert!(result.data["error"]
            .as_str()
            .unwrap()
            .contains("missing required field"));
    }
}
