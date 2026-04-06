use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

pub struct VerifyPlanExecutionTool;

#[async_trait]
impl ToolExecutor for VerifyPlanExecutionTool {
    fn name(&self) -> &str {
        "VerifyPlanExecution"
    }

    fn description(&self) -> String {
        "Verify that a plan was executed correctly by checking each planned step \
         against what was actually done. Call this after completing a multi-step \
         plan to ensure nothing was missed."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "planSteps": {
                    "type": "array",
                    "description": "The original plan steps to verify.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string", "description": "Step identifier." },
                            "description": { "type": "string", "description": "What the step was supposed to accomplish." },
                            "status": { "type": "string", "enum": ["completed", "partial", "skipped", "failed", "unknown"], "description": "Current assessment of step completion." },
                            "evidence": { "type": "string", "description": "Evidence that the step was completed." }
                        },
                        "required": ["id", "description", "status"]
                    }
                },
                "summary": { "type": "string", "description": "Overall summary of the plan execution." }
            },
            "required": ["planSteps"]
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
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let steps = match input.get("planSteps").and_then(|v| v.as_array()) {
            Some(s) => s,
            None => {
                return Ok(ToolResultData {
                    data: json!({ "error": "missing required field: planSteps (must be an array)" }),
                    is_error: true,
                })
            }
        };
        let summary = input.get("summary").and_then(|v| v.as_str()).unwrap_or("");

        let mut completed = 0u64;
        let mut partial = 0u64;
        let mut skipped = 0u64;
        let mut failed = 0u64;
        let mut unknown = 0u64;
        let mut verified_steps: Vec<Value> = Vec::new();

        for step in steps {
            let id = step.get("id").and_then(|v| v.as_str()).unwrap_or("?");
            let description = step
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let status = step
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let evidence = step.get("evidence").and_then(|v| v.as_str()).unwrap_or("");
            match status {
                "completed" => completed += 1,
                "partial" => partial += 1,
                "skipped" => skipped += 1,
                "failed" => failed += 1,
                _ => unknown += 1,
            }
            verified_steps.push(json!({ "id": id, "description": description, "status": status, "evidence": evidence, "verified": status == "completed" }));
        }

        let total = steps.len() as u64;
        let all_complete = completed == total;
        Ok(ToolResultData {
            data: json!({
                "verified": all_complete,
                "steps": verified_steps,
                "counts": { "total": total, "completed": completed, "partial": partial, "skipped": skipped, "failed": failed, "unknown": unknown },
                "summary": summary,
                "message": if all_complete { "All plan steps verified as completed.".to_string() }
                    else { format!("Plan verification: {}/{} completed, {} partial, {} skipped, {} failed.", completed, total, partial, skipped, failed) },
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
    async fn verify_all_completed() {
        let tool = VerifyPlanExecutionTool;
        let result = tool.call(&json!({
            "planSteps": [
                { "id": "1", "description": "Create file", "status": "completed", "evidence": "file.rs created" },
                { "id": "2", "description": "Add tests", "status": "completed", "evidence": "3 tests added" },
            ],
            "summary": "Feature implementation"
        }), &make_ctx(), CancellationToken::new(), None).await.unwrap();
        assert!(!result.is_error);
        assert!(result.data["verified"].as_bool().unwrap());
        assert_eq!(result.data["counts"]["completed"].as_u64().unwrap(), 2);
    }

    #[tokio::test]
    async fn verify_partial_completion() {
        let tool = VerifyPlanExecutionTool;
        let result = tool
            .call(
                &json!({
                    "planSteps": [
                        { "id": "1", "description": "Create file", "status": "completed" },
                        { "id": "2", "description": "Add tests", "status": "failed" },
                    ]
                }),
                &make_ctx(),
                CancellationToken::new(),
                None,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(!result.data["verified"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn verify_missing_steps() {
        let tool = VerifyPlanExecutionTool;
        let result = tool
            .call(&json!({}), &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();
        assert!(result.is_error);
    }
}
