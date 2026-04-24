use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

pub struct SuggestBackgroundPRTool;

#[async_trait]
impl ToolExecutor for SuggestBackgroundPRTool {
    fn name(&self) -> &str {
        "SuggestBackgroundPR"
    }

    fn description(&self) -> String {
        "Suggest creating a pull request in the background without blocking the \
         current conversation. The PR will be created by a background agent."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "Proposed PR title." },
                "body": { "type": "string", "description": "Proposed PR body/description in Markdown." },
                "branch": { "type": "string", "description": "Branch name for the PR." },
                "baseBranch": { "type": "string", "description": "Base branch to merge into." },
                "files": { "type": "array", "items": { "type": "string" }, "description": "List of file paths to include." },
                "draft": { "type": "boolean", "description": "Whether to create the PR as a draft. Defaults to false." }
            },
            "required": ["title", "body"]
        })
    }

    fn is_read_only(&self, _input: &Value) -> bool {
        false
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
        let title = match input.get("title").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => {
                return Ok(ToolResultData {
                    data: json!({ "error": "missing required field: title" }),
                    is_error: true,
                })
            }
        };
        let body = match input.get("body").and_then(|v| v.as_str()) {
            Some(b) => b,
            None => {
                return Ok(ToolResultData {
                    data: json!({ "error": "missing required field: body" }),
                    is_error: true,
                })
            }
        };
        let branch = input.get("branch").and_then(|v| v.as_str());
        let base_branch = input.get("baseBranch").and_then(|v| v.as_str());
        let files: Vec<String> = input
            .get("files")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        let draft = input
            .get("draft")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let suggestion_id = format!(
            "bg_pr_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );

        Ok(ToolResultData {
            data: json!({
                "suggested": true,
                "suggestionId": suggestion_id,
                "title": title,
                "body": body,
                "branch": branch,
                "baseBranch": base_branch,
                "files": if files.is_empty() { Value::Null } else { json!(files) },
                "draft": draft,
                "workingDirectory": ctx.working_directory.display().to_string(),
                "message": format!("Background PR suggestion recorded ({}). Title: '{}'", suggestion_id, title),
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
        ToolUseContext::for_test(
            PathBuf::from("/tmp/my-repo"),
            Arc::new(std::sync::Mutex::new(ReadFileState::new())),
            crate::registry::PermissionMode::Default,
        )
    }

    #[tokio::test]
    async fn suggest_pr_success() {
        let tool = SuggestBackgroundPRTool;
        let result = tool
            .call(
                &json!({ "title": "Add feature X", "body": "This PR adds feature X." }),
                &make_ctx(),
                CancellationToken::new(),
                None,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.data["suggested"].as_bool().unwrap());
        assert!(result.data["suggestionId"]
            .as_str()
            .unwrap()
            .starts_with("bg_pr_"));
    }

    #[tokio::test]
    async fn suggest_pr_missing_fields() {
        let tool = SuggestBackgroundPRTool;
        let ctx = make_ctx();
        let r = tool
            .call(
                &json!({ "body": "x" }),
                &ctx,
                CancellationToken::new(),
                None,
            )
            .await
            .unwrap();
        assert!(r.is_error);
        let r = tool
            .call(
                &json!({ "title": "x" }),
                &ctx,
                CancellationToken::new(),
                None,
            )
            .await
            .unwrap();
        assert!(r.is_error);
    }

    #[tokio::test]
    async fn suggest_pr_tool_properties() {
        let tool = SuggestBackgroundPRTool;
        assert_eq!(tool.name(), "SuggestBackgroundPR");
        assert!(!tool.is_read_only(&json!({})));
    }
}
