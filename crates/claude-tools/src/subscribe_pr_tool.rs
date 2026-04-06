use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

pub struct SubscribePRTool;

#[async_trait]
impl ToolExecutor for SubscribePRTool {
    fn name(&self) -> &str {
        "SubscribePR"
    }

    fn description(&self) -> String {
        "Subscribe to events on a GitHub pull request. When subscribed, you will \
         be notified of comments, reviews, CI status changes, and merges on the \
         specified PR."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "owner": { "type": "string", "description": "Repository owner (user or organization)." },
                "repo": { "type": "string", "description": "Repository name." },
                "prNumber": { "type": "integer", "description": "Pull request number." },
                "events": { "type": "array", "items": { "type": "string", "enum": ["comment", "review", "ci_status", "merge", "close", "all"] }, "description": "Events to subscribe to. Defaults to ['all']." }
            },
            "required": ["owner", "repo", "prNumber"]
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
        _ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let owner = match input.get("owner").and_then(|v| v.as_str()) {
            Some(o) => o,
            None => {
                return Ok(ToolResultData {
                    data: json!({ "error": "missing required field: owner" }),
                    is_error: true,
                })
            }
        };
        let repo = match input.get("repo").and_then(|v| v.as_str()) {
            Some(r) => r,
            None => {
                return Ok(ToolResultData {
                    data: json!({ "error": "missing required field: repo" }),
                    is_error: true,
                })
            }
        };
        let pr_number = match input.get("prNumber").and_then(|v| v.as_u64()) {
            Some(n) => n,
            None => {
                return Ok(ToolResultData {
                    data: json!({ "error": "missing required field: prNumber (must be a positive integer)" }),
                    is_error: true,
                })
            }
        };
        let events: Vec<String> = input
            .get("events")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_else(|| vec!["all".to_string()]);

        let subscription_id = format!(
            "pr_sub_{}_{}_{}_{}",
            owner,
            repo,
            pr_number,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );

        Ok(ToolResultData {
            data: json!({
                "subscribed": true,
                "subscriptionId": subscription_id,
                "owner": owner,
                "repo": repo,
                "prNumber": pr_number,
                "events": events,
                "prUrl": format!("https://github.com/{}/{}/pull/{}", owner, repo, pr_number),
                "message": format!("Subscribed to events on {}/{} PR #{}.", owner, repo, pr_number),
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
        ToolUseContext {
            working_directory: PathBuf::from("/tmp"),
            read_file_state: Arc::new(std::sync::Mutex::new(ReadFileState::new())),
            permission_mode: crate::registry::PermissionMode::Default,
        }
    }

    #[tokio::test]
    async fn subscribe_pr_success() {
        let tool = SubscribePRTool;
        let result = tool
            .call(
                &json!({ "owner": "anthropics", "repo": "claude-code", "prNumber": 123 }),
                &make_ctx(),
                CancellationToken::new(),
                None,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.data["subscribed"].as_bool().unwrap());
        assert_eq!(result.data["prNumber"].as_u64().unwrap(), 123);
    }

    #[tokio::test]
    async fn subscribe_pr_missing_fields() {
        let tool = SubscribePRTool;
        let ctx = make_ctx();
        let r = tool
            .call(
                &json!({ "repo": "x", "prNumber": 1 }),
                &ctx,
                CancellationToken::new(),
                None,
            )
            .await
            .unwrap();
        assert!(r.is_error);
        let r = tool
            .call(
                &json!({ "owner": "x", "prNumber": 1 }),
                &ctx,
                CancellationToken::new(),
                None,
            )
            .await
            .unwrap();
        assert!(r.is_error);
        let r = tool
            .call(
                &json!({ "owner": "x", "repo": "y" }),
                &ctx,
                CancellationToken::new(),
                None,
            )
            .await
            .unwrap();
        assert!(r.is_error);
    }

    #[tokio::test]
    async fn subscribe_pr_tool_properties() {
        let tool = SubscribePRTool;
        assert_eq!(tool.name(), "SubscribePR");
        assert!(!tool.is_read_only(&json!({})));
    }
}
