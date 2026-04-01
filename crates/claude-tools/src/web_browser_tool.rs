use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

pub struct WebBrowserTool;

#[async_trait]
impl ToolExecutor for WebBrowserTool {
    fn name(&self) -> &str { "WebBrowser" }

    fn description(&self) -> String {
        "Control a web browser for tasks like reading pages, filling forms, \
         and taking screenshots. This tool requires the Claude-in-Chrome browser \
         extension to be installed and connected. For basic web page reading, \
         prefer WebFetch instead.".to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["navigate", "read_page", "click", "type", "screenshot", "evaluate", "get_tabs", "close_tab"], "description": "The browser action to perform." },
                "url": { "type": "string", "description": "URL to navigate to (for 'navigate' action)." },
                "selector": { "type": "string", "description": "CSS selector for element interaction." },
                "text": { "type": "string", "description": "Text to type (for 'type' action)." },
                "script": { "type": "string", "description": "JavaScript to evaluate (for 'evaluate' action)." }
            },
            "required": ["action"]
        })
    }

    fn is_read_only(&self, input: &Value) -> bool {
        matches!(input.get("action").and_then(|v| v.as_str()), Some("read_page" | "screenshot" | "get_tabs"))
    }
    fn is_concurrency_safe(&self, _input: &Value) -> bool { false }

    async fn call(&self, input: &Value, _ctx: &ToolUseContext, _cancel: CancellationToken, _progress: Option<ProgressSender>) -> Result<ToolResultData> {
        let action = match input.get("action").and_then(|v| v.as_str()) {
            Some(a) => a,
            None => return Ok(ToolResultData { data: json!({ "error": "missing required field: action" }), is_error: true }),
        };
        Ok(ToolResultData {
            data: json!({
                "action": action,
                "available": false,
                "message": format!(
                    "WebBrowser action '{}' requires the Claude-in-Chrome browser extension. \
                     The extension is not currently connected. For basic web page reading, use WebFetch.", action),
                "alternatives": ["WebFetch", "mcp__claude-in-chrome__navigate", "mcp__claude-in-chrome__read_page"],
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
        ToolUseContext { working_directory: PathBuf::from("/tmp"), read_file_state: Arc::new(std::sync::Mutex::new(ReadFileState::new())) }
    }

    #[tokio::test]
    async fn web_browser_missing_action() {
        let tool = WebBrowserTool;
        let result = tool.call(&json!({}), &make_ctx(), CancellationToken::new(), None).await.unwrap();
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn web_browser_returns_extension_message() {
        let tool = WebBrowserTool;
        let result = tool.call(&json!({ "action": "navigate" }), &make_ctx(), CancellationToken::new(), None).await.unwrap();
        assert!(!result.is_error);
        assert!(!result.data["available"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn web_browser_tool_properties() {
        let tool = WebBrowserTool;
        assert_eq!(tool.name(), "WebBrowser");
        assert!(tool.is_read_only(&json!({ "action": "read_page" })));
        assert!(!tool.is_read_only(&json!({ "action": "navigate" })));
    }
}
