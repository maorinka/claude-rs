use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

const MAX_JOBS: usize = 50;

pub struct ScheduleCronTool;

/// Validate a 5-field cron expression: M H DoM Mon DoW
fn validate_cron_expression(expr: &str) -> bool {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    if fields.len() != 5 {
        return false;
    }

    fn validate_field(field: &str, min: u32, max: u32) -> bool {
        if field == "*" {
            return true;
        }
        // Handle */N
        if let Some(step) = field.strip_prefix("*/") {
            return step.parse::<u32>().map_or(false, |n| n > 0 && n <= max);
        }
        // Handle comma-separated values
        for part in field.split(',') {
            // Handle ranges like 1-5
            if part.contains('-') {
                let range: Vec<&str> = part.split('-').collect();
                if range.len() != 2 {
                    return false;
                }
                let start = match range[0].parse::<u32>() {
                    Ok(v) => v,
                    Err(_) => return false,
                };
                let end = match range[1].parse::<u32>() {
                    Ok(v) => v,
                    Err(_) => return false,
                };
                if start < min || end > max || start > end {
                    return false;
                }
            } else {
                match part.parse::<u32>() {
                    Ok(v) if v >= min && v <= max => {}
                    _ => return false,
                }
            }
        }
        true
    }

    validate_field(fields[0], 0, 59)   // minute
        && validate_field(fields[1], 0, 23)   // hour
        && validate_field(fields[2], 1, 31)   // day of month
        && validate_field(fields[3], 1, 12)   // month
        && validate_field(fields[4], 0, 7) // day of week (0 and 7 = Sunday)
}

#[async_trait]
impl ToolExecutor for ScheduleCronTool {
    fn name(&self) -> &str {
        "ScheduleCron"
    }

    fn description(&self) -> String {
        r#"Schedule a recurring or one-shot cron task. Persists configuration to ~/.claude/cron/.

Parameters:
- cron (required): Standard 5-field cron expression in local time: "M H DoM Mon DoW" (e.g. "*/5 * * * *" = every 5 minutes).
- prompt (required): The prompt to enqueue at each fire time.
- name (optional): A name for the cron job. If not provided, a UUID is generated.
- recurring (optional, default true): true = fire on every cron match; false = fire once then auto-delete."#
            .to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "cron": {
                    "type": "string",
                    "description": "Standard 5-field cron expression: M H DoM Mon DoW"
                },
                "prompt": {
                    "type": "string",
                    "description": "The prompt to enqueue at each fire time."
                },
                "name": {
                    "type": "string",
                    "description": "Optional name for the cron job."
                },
                "recurring": {
                    "type": "boolean",
                    "description": "true (default) = fire on every cron match; false = fire once then auto-delete."
                }
            },
            "required": ["cron", "prompt"]
        })
    }

    async fn call(
        &self,
        input: &Value,
        _ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let cron_expr = match input.get("cron").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => {
                return Ok(ToolResultData {
                    data: json!({ "error": "missing required field: cron" }),
                    is_error: true,
                });
            }
        };

        let prompt = match input.get("prompt").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => {
                return Ok(ToolResultData {
                    data: json!({ "error": "missing required field: prompt" }),
                    is_error: true,
                });
            }
        };

        if !validate_cron_expression(cron_expr) {
            return Ok(ToolResultData {
                data: json!({
                    "error": format!("Invalid cron expression '{}'. Expected 5 fields: M H DoM Mon DoW.", cron_expr)
                }),
                is_error: true,
            });
        }

        let name = input
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        let recurring = input
            .get("recurring")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        // Persist cron config to ~/.claude/cron/<name>.json
        let home = match dirs_home() {
            Some(h) => h,
            None => {
                return Ok(ToolResultData {
                    data: json!({ "error": "Could not determine home directory" }),
                    is_error: true,
                });
            }
        };

        let cron_dir = home.join(".claude").join("cron");
        if let Err(e) = tokio::fs::create_dir_all(&cron_dir).await {
            return Ok(ToolResultData {
                data: json!({ "error": format!("Failed to create cron directory: {}", e) }),
                is_error: true,
            });
        }

        // Check max jobs
        let mut count = 0;
        if let Ok(mut entries) = tokio::fs::read_dir(&cron_dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                if entry.path().extension().map_or(false, |ext| ext == "json") {
                    count += 1;
                }
            }
        }

        if count >= MAX_JOBS {
            return Ok(ToolResultData {
                data: json!({
                    "error": format!("Too many scheduled jobs (max {}). Cancel one first.", MAX_JOBS)
                }),
                is_error: true,
            });
        }

        let config = json!({
            "id": name,
            "cron": cron_expr,
            "prompt": prompt,
            "recurring": recurring,
            "created_at": chrono::Utc::now().to_rfc3339(),
        });

        let file_path = cron_dir.join(format!("{}.json", name));
        if let Err(e) = tokio::fs::write(&file_path, serde_json::to_string_pretty(&config)?).await {
            return Ok(ToolResultData {
                data: json!({ "error": format!("Failed to write cron config: {}", e) }),
                is_error: true,
            });
        }

        let human_schedule = cron_to_human(cron_expr);

        Ok(ToolResultData {
            data: json!({
                "id": name,
                "humanSchedule": human_schedule,
                "recurring": recurring,
                "message": if recurring {
                    format!("Scheduled recurring job {} ({}). Use CronDelete to cancel.", name, human_schedule)
                } else {
                    format!("Scheduled one-shot task {} ({}). It will fire once then auto-delete.", name, human_schedule)
                }
            }),
            is_error: false,
        })
    }
}

fn dirs_home() -> Option<std::path::PathBuf> {
    #[cfg(unix)]
    {
        std::env::var_os("HOME").map(std::path::PathBuf::from)
    }
    #[cfg(windows)]
    {
        std::env::var_os("USERPROFILE").map(std::path::PathBuf::from)
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}

/// Convert a cron expression to a human-readable string
fn cron_to_human(expr: &str) -> String {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    if fields.len() != 5 {
        return expr.to_string();
    }

    let (minute, hour, dom, month, dow) = (fields[0], fields[1], fields[2], fields[3], fields[4]);

    // Handle common patterns
    if minute == "*" && hour == "*" && dom == "*" && month == "*" && dow == "*" {
        return "every minute".to_string();
    }
    if let Some(step) = minute.strip_prefix("*/") {
        if hour == "*" && dom == "*" && month == "*" && dow == "*" {
            return format!("every {} minutes", step);
        }
    }
    if minute != "*" && hour != "*" && dom == "*" && month == "*" && dow == "*" {
        return format!("daily at {}:{:0>2}", hour, minute);
    }

    format!("cron({} {} {} {} {})", minute, hour, dom, month, dow)
}

// ---------------------------------------------------------------------------
// CronDeleteTool — cancel a scheduled cron job by id
// ---------------------------------------------------------------------------

pub struct CronDeleteTool;

#[async_trait]
impl ToolExecutor for CronDeleteTool {
    fn name(&self) -> &str {
        "CronDelete"
    }

    fn description(&self) -> String {
        "Cancel a scheduled cron job by its ID. The job's JSON configuration file \
         is removed from ~/.claude/cron/."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Job ID returned by ScheduleCron."
                }
            },
            "required": ["id"]
        })
    }

    async fn call(
        &self,
        input: &Value,
        _ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let id = match input.get("id").and_then(|v| v.as_str()) {
            Some(i) => i,
            None => {
                return Ok(ToolResultData {
                    data: json!({ "error": "missing required field: id" }),
                    is_error: true,
                });
            }
        };

        // Reject path traversal
        if id.contains("..") || id.contains('/') || id.contains('\\') {
            return Ok(ToolResultData {
                data: json!({ "error": "Invalid job id: must not contain path separators or '..'" }),
                is_error: true,
            });
        }

        let home = match dirs_home() {
            Some(h) => h,
            None => {
                return Ok(ToolResultData {
                    data: json!({ "error": "Could not determine home directory" }),
                    is_error: true,
                });
            }
        };

        let file_path = home
            .join(".claude")
            .join("cron")
            .join(format!("{}.json", id));
        if !file_path.exists() {
            return Ok(ToolResultData {
                data: json!({ "error": format!("No scheduled job with id '{}'", id) }),
                is_error: true,
            });
        }

        if let Err(e) = tokio::fs::remove_file(&file_path).await {
            return Ok(ToolResultData {
                data: json!({ "error": format!("Failed to remove cron config: {}", e) }),
                is_error: true,
            });
        }

        Ok(ToolResultData {
            data: json!({ "id": id, "message": format!("Cancelled job {}.", id) }),
            is_error: false,
        })
    }
}

// ---------------------------------------------------------------------------
// CronListTool — list all active cron jobs
// ---------------------------------------------------------------------------

pub struct CronListTool;

#[async_trait]
impl ToolExecutor for CronListTool {
    fn name(&self) -> &str {
        "CronList"
    }

    fn description(&self) -> String {
        "List all active scheduled cron jobs. Returns each job's id, cron expression, \
         human-readable schedule, prompt, and whether it is recurring."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "required": []
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
        let home = match dirs_home() {
            Some(h) => h,
            None => {
                return Ok(ToolResultData {
                    data: json!({ "jobs": [], "count": 0, "message": "Could not determine home directory." }),
                    is_error: false,
                });
            }
        };

        let cron_dir = home.join(".claude").join("cron");
        let mut jobs: Vec<Value> = Vec::new();

        let mut entries = match tokio::fs::read_dir(&cron_dir).await {
            Ok(e) => e,
            Err(_) => {
                return Ok(ToolResultData {
                    data: json!({ "jobs": [], "count": 0, "message": "No scheduled jobs." }),
                    is_error: false,
                });
            }
        };

        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.extension().map_or(true, |ext| ext != "json") {
                continue;
            }
            if let Ok(content) = tokio::fs::read_to_string(&path).await {
                if let Ok(data) = serde_json::from_str::<Value>(&content) {
                    let id = data
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?")
                        .to_string();
                    let cron_expr = data
                        .get("cron")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let prompt = data
                        .get("prompt")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let recurring = data
                        .get("recurring")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true);
                    let human_schedule = cron_to_human(&cron_expr);
                    jobs.push(json!({
                        "id": id,
                        "cron": cron_expr,
                        "humanSchedule": human_schedule,
                        "prompt": prompt,
                        "recurring": recurring,
                    }));
                }
            }
        }

        jobs.sort_by(|a, b| {
            a["id"]
                .as_str()
                .unwrap_or("")
                .cmp(b["id"].as_str().unwrap_or(""))
        });
        let count = jobs.len();

        Ok(ToolResultData {
            data: json!({
                "jobs": jobs,
                "count": count,
                "message": if count == 0 { "No scheduled jobs.".to_string() }
                    else { format!("{} scheduled job(s).", count) },
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

    #[test]
    fn validate_cron_valid_expressions() {
        assert!(validate_cron_expression("*/5 * * * *"));
        assert!(validate_cron_expression("0 12 * * *"));
        assert!(validate_cron_expression("30 14 28 2 *"));
        assert!(validate_cron_expression("0 0 1 1 0"));
        assert!(validate_cron_expression("0,30 * * * *"));
        assert!(validate_cron_expression("0 9-17 * * 1-5"));
    }

    #[test]
    fn validate_cron_invalid_expressions() {
        assert!(!validate_cron_expression(""));
        assert!(!validate_cron_expression("* * *"));
        assert!(!validate_cron_expression("60 * * * *"));
        assert!(!validate_cron_expression("* 25 * * *"));
        assert!(!validate_cron_expression("* * 32 * *"));
        assert!(!validate_cron_expression("* * * 13 *"));
        assert!(!validate_cron_expression("abc * * * *"));
    }

    #[tokio::test]
    async fn schedule_cron_missing_fields() {
        let tool = ScheduleCronTool;
        let ctx = make_ctx();
        let cancel = CancellationToken::new();

        let result = tool
            .call(&json!({}), &ctx, cancel.clone(), None)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.data["error"].as_str().unwrap().contains("cron"));

        let result = tool
            .call(&json!({ "cron": "*/5 * * * *" }), &ctx, cancel, None)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.data["error"].as_str().unwrap().contains("prompt"));
    }

    #[tokio::test]
    async fn schedule_cron_invalid_expression() {
        let tool = ScheduleCronTool;
        let input = json!({ "cron": "bad expr", "prompt": "test" });
        let ctx = make_ctx();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(result.is_error);
        assert!(result.data["error"]
            .as_str()
            .unwrap()
            .contains("Invalid cron expression"));
    }

    #[tokio::test]
    async fn schedule_cron_success() {
        let tool = ScheduleCronTool;
        let input = json!({
            "cron": "*/5 * * * *",
            "prompt": "check status",
            "name": "test-cron-job"
        });
        let ctx = make_ctx();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(!result.is_error);
        assert_eq!(result.data["id"].as_str().unwrap(), "test-cron-job");
        assert!(result.data["recurring"].as_bool().unwrap());

        // Clean up
        if let Some(home) = dirs_home() {
            let file_path = home.join(".claude").join("cron").join("test-cron-job.json");
            let _ = tokio::fs::remove_file(file_path).await;
        }
    }

    #[test]
    fn cron_to_human_common_patterns() {
        assert_eq!(cron_to_human("* * * * *"), "every minute");
        assert_eq!(cron_to_human("*/5 * * * *"), "every 5 minutes");
        assert_eq!(cron_to_human("30 14 * * *"), "daily at 14:30");
    }

    // --- CronDeleteTool tests ---

    #[tokio::test]
    async fn cron_delete_missing_id() {
        let tool = CronDeleteTool;
        let result = tool
            .call(&json!({}), &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.data["error"].as_str().unwrap().contains("id"));
    }

    #[tokio::test]
    async fn cron_delete_path_traversal_rejected() {
        let tool = CronDeleteTool;
        let result = tool
            .call(
                &json!({ "id": "../etc/passwd" }),
                &make_ctx(),
                CancellationToken::new(),
                None,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.data["error"].as_str().unwrap().contains("Invalid"));
    }

    #[tokio::test]
    async fn cron_delete_nonexistent_job() {
        let tool = CronDeleteTool;
        let result = tool
            .call(
                &json!({ "id": "nonexistent-job-12345" }),
                &make_ctx(),
                CancellationToken::new(),
                None,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.data["error"]
            .as_str()
            .unwrap()
            .contains("No scheduled job"));
    }

    #[tokio::test]
    async fn cron_delete_properties() {
        let tool = CronDeleteTool;
        assert_eq!(tool.name(), "CronDelete");
    }

    // --- CronListTool tests ---

    #[tokio::test]
    async fn cron_list_empty() {
        // When no cron dir exists, should return empty list
        let tool = CronListTool;
        let result = tool
            .call(&json!({}), &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert_eq!(
            result.data["count"].as_u64().unwrap_or(0),
            result.data["jobs"]
                .as_array()
                .map(|a| a.len() as u64)
                .unwrap_or(0)
        );
    }

    #[tokio::test]
    async fn cron_list_properties() {
        let tool = CronListTool;
        assert_eq!(tool.name(), "CronList");
        assert!(tool.is_read_only(&json!({})));
        assert!(tool.is_concurrency_safe(&json!({})));
    }
}
