use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

pub struct AgentTool;

#[async_trait]
impl ToolExecutor for AgentTool {
    fn name(&self) -> &str { "Agent" }
    fn aliases(&self) -> &[&str] { &["agent"] }
    fn input_schema(&self) -> Value {
        json!({"type":"object","required":["prompt"],"properties":{"prompt":{"type":"string"},"description":{"type":"string"},"model":{"type":"string"},"subagent_type":{"type":"string"},"run_in_background":{"type":"boolean"},"isolation":{"type":"string","enum":["worktree"]}}})
    }
    async fn call(&self, input: &Value, ctx: &ToolUseContext, cancel: CancellationToken, _progress: Option<ProgressSender>) -> Result<ToolResultData> {
        let prompt = input["prompt"].as_str().unwrap_or("");
        let model = input.get("model").and_then(|v| v.as_str());
        // Spawn child claude-rs process
        let mut cmd = tokio::process::Command::new(std::env::current_exe()?);
        cmd.arg("-p").arg(prompt);
        if let Some(m) = model { cmd.arg("--model").arg(m); }
        cmd.current_dir(&ctx.working_directory);
        cmd.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
        let output = cmd.output().await?;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        Ok(ToolResultData { data: json!({"status":"completed","prompt":prompt,"result":stdout}), is_error: !output.status.success() })
    }
}
