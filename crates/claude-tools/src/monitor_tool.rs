use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

pub struct MonitorTool;

#[async_trait]
impl ToolExecutor for MonitorTool {
    fn name(&self) -> &str {
        "Monitor"
    }
    fn description(&self) -> String {
        "Monitor system resources. Returns current CPU usage, memory utilization, and disk usage for the working directory's filesystem.".to_string()
    }
    fn input_schema(&self) -> Value {
        json!({ "type": "object", "properties": { "resource": { "type": "string", "enum": ["all", "cpu", "memory", "disk"], "description": "Which resource to monitor. Defaults to 'all'." } }, "required": [] })
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
        let resource = input
            .get("resource")
            .and_then(|v| v.as_str())
            .unwrap_or("all");
        let mut result = json!({});
        if resource == "all" || resource == "cpu" {
            let cpu = if let Ok(out) = tokio::process::Command::new("sysctl")
                .args(["-n", "hw.ncpu"])
                .output()
                .await
            {
                let ncpu = String::from_utf8_lossy(&out.stdout)
                    .trim()
                    .parse::<u32>()
                    .unwrap_or(0);
                let load = if let Ok(lo) = tokio::process::Command::new("sysctl")
                    .args(["-n", "vm.loadavg"])
                    .output()
                    .await
                {
                    let raw = String::from_utf8_lossy(&lo.stdout).trim().to_string();
                    let parts: Vec<f64> = raw
                        .trim_matches(|c: char| c == '{' || c == '}' || c.is_whitespace())
                        .split_whitespace()
                        .filter_map(|s| s.parse().ok())
                        .collect();
                    json!({ "load1m": parts.first().copied().unwrap_or(0.0), "load5m": parts.get(1).copied().unwrap_or(0.0), "load15m": parts.get(2).copied().unwrap_or(0.0) })
                } else {
                    json!({})
                };
                json!({ "cores": ncpu, "loadAverage": load })
            } else {
                json!({ "error": "unable to read CPU info" })
            };
            result["cpu"] = cpu;
        }
        if resource == "all" || resource == "memory" {
            let mem = if let Ok(out) = tokio::process::Command::new("sysctl")
                .args(["-n", "hw.memsize"])
                .output()
                .await
            {
                let total = String::from_utf8_lossy(&out.stdout)
                    .trim()
                    .parse::<u64>()
                    .unwrap_or(0);
                let total_mb = total / (1024 * 1024);
                json!({ "totalMB": total_mb })
            } else {
                json!({ "error": "unable to read memory info" })
            };
            result["memory"] = mem;
        }
        if resource == "all" || resource == "disk" {
            let dir_str = ctx.working_directory.to_string_lossy().to_string();
            let disk = if let Ok(out) = tokio::process::Command::new("df")
                .args(["-k", &dir_str])
                .output()
                .await
            {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let lines: Vec<&str> = stdout.lines().collect();
                if lines.len() >= 2 {
                    let parts: Vec<&str> = lines[1].split_whitespace().collect();
                    if parts.len() >= 5 {
                        let total_kb: u64 = parts[1].parse().unwrap_or(0);
                        let used_kb: u64 = parts[2].parse().unwrap_or(0);
                        let avail_kb: u64 = parts[3].parse().unwrap_or(0);
                        json!({ "filesystem": parts[0], "totalGB": format!("{:.1}", total_kb as f64 / 1048576.0), "usedGB": format!("{:.1}", used_kb as f64 / 1048576.0), "availableGB": format!("{:.1}", avail_kb as f64 / 1048576.0), "usagePercent": parts[4].trim_end_matches('%') })
                    } else {
                        json!({ "error": "unable to parse df output" })
                    }
                } else {
                    json!({ "error": "unable to read disk info" })
                }
            } else {
                json!({ "error": "df command failed" })
            };
            result["disk"] = disk;
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
        ToolUseContext::for_test(
            PathBuf::from("/tmp"),
            Arc::new(std::sync::Mutex::new(ReadFileState::new())),
            crate::registry::PermissionMode::Default,
        )
    }
    #[tokio::test]
    async fn monitor_all_resources() {
        let r = MonitorTool
            .call(&json!({}), &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();
        assert!(!r.is_error);
        assert!(r.data.get("cpu").is_some());
        assert!(r.data.get("memory").is_some());
        assert!(r.data.get("disk").is_some());
    }
    #[tokio::test]
    async fn monitor_cpu_only() {
        let r = MonitorTool
            .call(
                &json!({ "resource": "cpu" }),
                &make_ctx(),
                CancellationToken::new(),
                None,
            )
            .await
            .unwrap();
        assert!(!r.is_error);
        assert!(r.data.get("cpu").is_some());
        assert!(r.data.get("memory").is_none());
    }
    #[tokio::test]
    async fn monitor_tool_properties() {
        assert_eq!(MonitorTool.name(), "Monitor");
        assert!(MonitorTool.is_read_only(&json!({})));
    }
}
