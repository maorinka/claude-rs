use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

pub struct CtxInspectTool;

#[async_trait]
impl ToolExecutor for CtxInspectTool {
    fn name(&self) -> &str {
        "CtxInspect"
    }
    fn description(&self) -> String {
        "Inspect the current context window. Returns metadata about the conversation state including the working directory and files that have been read.".to_string()
    }
    fn input_schema(&self) -> Value {
        json!({ "type": "object", "properties": { "section": { "type": "string", "enum": ["all", "files", "tools", "summary"], "description": "Which section of context to inspect. Defaults to 'summary'." } }, "required": [] })
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
        ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let section = input
            .get("section")
            .and_then(|v| v.as_str())
            .unwrap_or("summary");
        let mut result = json!({});
        if section == "all" || section == "summary" {
            result["workingDirectory"] = json!(ctx.working_directory.display().to_string());
            result["summary"] = json!({ "message": "Context inspection provides a snapshot of the current session state." });
        }
        if section == "all" || section == "files" {
            result["files"] = json!([]);
        }
        if section == "all" || section == "tools" {
            result["tools"] = json!({ "note": "Tool enumeration is handled by the engine. Use ToolSearch to find specific tools." });
        }
        Ok(ToolResultData {
            data: result,
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
        ToolUseContext {
            working_directory: PathBuf::from("/tmp/test-project"),
            read_file_state: Arc::new(std::sync::Mutex::new(ReadFileState::new())),
            permission_mode: crate::registry::PermissionMode::Default,
            ..Default::default()
        }
    }
    #[tokio::test]
    async fn ctx_inspect_summary() {
        let r = CtxInspectTool
            .call(&json!({}), &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();
        assert!(!r.is_error);
        assert_eq!(
            r.data["workingDirectory"].as_str().unwrap(),
            "/tmp/test-project"
        );
    }
    #[tokio::test]
    async fn ctx_inspect_all() {
        let r = CtxInspectTool
            .call(
                &json!({ "section": "all" }),
                &make_ctx(),
                CancellationToken::new(),
                None,
            )
            .await
            .unwrap();
        assert!(!r.is_error);
        assert!(r.data.get("files").is_some());
        assert!(r.data.get("tools").is_some());
    }
    #[tokio::test]
    async fn ctx_inspect_properties() {
        assert_eq!(CtxInspectTool.name(), "CtxInspect");
        assert!(CtxInspectTool.is_read_only(&json!({})));
    }
}
