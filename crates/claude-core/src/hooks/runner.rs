use anyhow::Result;
use std::process::Stdio;
use super::types::{HookConfig, HookEvent};

pub struct HookRunner {
    hooks: Vec<HookConfig>,
}

impl HookRunner {
    pub fn new(hooks: Vec<HookConfig>) -> Self { Self { hooks } }

    pub fn from_settings(settings: &serde_json::Value) -> Self {
        let hooks = settings.get("hooks")
            .and_then(|h| serde_json::from_value::<Vec<HookConfig>>(h.clone()).ok())
            .unwrap_or_default();
        Self { hooks }
    }

    pub async fn run_hooks(&self, event: HookEvent, env: &[(&str, &str)]) -> Result<Vec<HookResult>> {
        let mut results = Vec::new();
        for hook in &self.hooks {
            if hook.event == event {
                let result = run_single_hook(hook, env).await?;
                results.push(result);
            }
        }
        Ok(results)
    }
}

pub struct HookResult {
    pub command: String,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

async fn run_single_hook(hook: &HookConfig, env: &[(&str, &str)]) -> Result<HookResult> {
    let mut cmd = tokio::process::Command::new("bash");
    cmd.arg("-c").arg(&hook.command);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    for (k, v) in env { cmd.env(k, v); }

    let timeout = std::time::Duration::from_millis(hook.timeout_ms.unwrap_or(30_000));
    let output = tokio::time::timeout(timeout, cmd.output()).await??;

    Ok(HookResult {
        command: hook.command.clone(),
        exit_code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}
