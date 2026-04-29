use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

pub struct SyntheticOutputTool;

impl SyntheticOutputTool {
    pub fn new() -> Self {
        Self
    }
}

pub struct JsonSchemaSyntheticOutputTool {
    schema: Value,
    validator: jsonschema::Validator,
}

impl JsonSchemaSyntheticOutputTool {
    pub fn new(schema: Value) -> Result<Self> {
        let validator = jsonschema::validator_for(&schema)
            .map_err(|err| anyhow::anyhow!("Invalid JSON schema: {err}"))?;
        Ok(Self { schema, validator })
    }
}

#[async_trait]
impl ToolExecutor for SyntheticOutputTool {
    fn name(&self) -> &str {
        "StructuredOutput"
    }

    fn description(&self) -> String {
        "Return structured output in the requested format. Use this tool to return your final \
         response in the requested structured format. You MUST call this tool exactly once at \
         the end of your response to provide the structured output."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        // Accept any object since the schema is provided dynamically
        json!({
            "type": "object",
            "additionalProperties": true,
            "description": "The structured output data to return."
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
        // The tool just validates and returns the input as the structured output.
        // In the full implementation, this would validate against a provided JSON schema.
        Ok(ToolResultData {
            data: json!({
                "message": "Structured output provided successfully",
                "structured_output": input
            }),
            is_error: false,
        })
    }
}

#[async_trait]
impl ToolExecutor for JsonSchemaSyntheticOutputTool {
    fn name(&self) -> &str {
        "StructuredOutput"
    }

    fn description(&self) -> String {
        "Return structured output in the requested format. Use this tool to return your final \
         response in the requested structured format. You MUST call this tool exactly once at \
         the end of your response to provide the structured output."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        self.schema.clone()
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
        if let Err(error) = self.validator.validate(input) {
            let errors = std::iter::once(error)
                .map(|err| {
                    let path = err.instance_path().to_string();
                    let path = if path.is_empty() {
                        "root".to_string()
                    } else {
                        path
                    };
                    format!("{path}: {err}")
                })
                .collect::<Vec<_>>()
                .join(", ");
            anyhow::bail!("Output does not match required schema: {errors}");
        }
        Ok(ToolResultData {
            data: json!({
                "message": "Structured output provided successfully",
                "structured_output": input
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
            PathBuf::from("/tmp"),
            Arc::new(std::sync::Mutex::new(ReadFileState::new())),
            crate::registry::PermissionMode::Default,
        )
    }

    #[tokio::test]
    async fn synthetic_output_returns_input() {
        let tool = SyntheticOutputTool;
        let input = json!({
            "title": "Bug Report",
            "severity": "high",
            "count": 3
        });
        let ctx = make_ctx();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(!result.is_error);
        assert_eq!(
            result.data["structured_output"]["title"].as_str().unwrap(),
            "Bug Report"
        );
        assert_eq!(
            result.data["structured_output"]["severity"]
                .as_str()
                .unwrap(),
            "high"
        );
        assert_eq!(
            result.data["structured_output"]["count"].as_u64().unwrap(),
            3
        );
    }

    #[tokio::test]
    async fn synthetic_output_empty_input() {
        let tool = SyntheticOutputTool;
        let input = json!({});
        let ctx = make_ctx();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(!result.is_error);
        assert_eq!(
            result.data["message"].as_str().unwrap(),
            "Structured output provided successfully"
        );
    }

    #[tokio::test]
    async fn synthetic_output_validates_json_schema() {
        let tool = JsonSchemaSyntheticOutputTool::new(json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"}
            },
            "required": ["name"]
        }))
        .unwrap();
        let ctx = make_ctx();

        let ok = tool
            .call(
                &json!({"name": "Claude"}),
                &ctx,
                CancellationToken::new(),
                None,
            )
            .await
            .unwrap();
        assert_eq!(ok.data["structured_output"]["name"], "Claude");

        let err = tool
            .call(&json!({"name": 42}), &ctx, CancellationToken::new(), None)
            .await
            .unwrap_err();
        assert!(err
            .to_string()
            .contains("Output does not match required schema"));
    }

    #[tokio::test]
    async fn synthetic_output_is_read_only() {
        let tool = SyntheticOutputTool;
        assert!(tool.is_read_only(&json!({})));
        assert!(tool.is_concurrency_safe(&json!({})));
    }
}
