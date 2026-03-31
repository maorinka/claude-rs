use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

pub struct LSPTool;

#[async_trait]
impl ToolExecutor for LSPTool {
    fn name(&self) -> &str {
        "LSP"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "The LSP action to perform (e.g. 'diagnostics', 'hover', 'completion')."
                },
                "file_path": {
                    "type": "string",
                    "description": "The file path to run the LSP action on."
                }
            },
            "required": ["action"]
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
        // Stub: LSP integration is not yet implemented; return empty diagnostics.
        Ok(ToolResultData {
            data: json!({ "diagnostics": [] }),
            is_error: false,
        })
    }
}
