use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

pub struct PushNotificationTool;

#[async_trait]
impl ToolExecutor for PushNotificationTool {
    fn name(&self) -> &str {
        "PushNotification"
    }
    fn description(&self) -> String {
        "Send a desktop notification to the user. Use this when a long-running task completes or when you need the user's attention. The notification appears as a native OS notification (macOS or Linux).".to_string()
    }
    fn input_schema(&self) -> Value {
        json!({ "type": "object", "properties": { "title": { "type": "string", "description": "The notification title." }, "body": { "type": "string", "description": "The notification body text." } }, "required": ["title", "body"] })
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
        let _title = match input.get("title").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => {
                return Ok(ToolResultData {
                    data: json!({ "error": "missing required field: title" }),
                    is_error: true,
                })
            }
        };
        let _body = match input.get("body").and_then(|v| v.as_str()) {
            Some(b) => b,
            None => {
                return Ok(ToolResultData {
                    data: json!({ "error": "missing required field: body" }),
                    is_error: true,
                })
            }
        };
        // Alias without underscore for platform-specific blocks
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        let (title, body) = (_title, _body);
        #[cfg(target_os = "macos")]
        {
            let script = format!(
                r#"display notification "{}" with title "{}""#,
                body.replace('\\', "\\\\").replace('"', "\\\""),
                title.replace('\\', "\\\\").replace('"', "\\\"")
            );
            match tokio::process::Command::new("osascript")
                .args(["-e", &script])
                .output()
                .await
            {
                Ok(out) if out.status.success() => {
                    return Ok(ToolResultData {
                        data: json!({ "sent": true, "title": title, "message": "Notification sent successfully." }),
                        is_error: false,
                    })
                }
                Ok(out) => {
                    return Ok(ToolResultData {
                        data: json!({ "sent": false, "error": format!("osascript failed: {}", String::from_utf8_lossy(&out.stderr)) }),
                        is_error: true,
                    })
                }
                Err(e) => {
                    return Ok(ToolResultData {
                        data: json!({ "sent": false, "error": format!("Failed to run osascript: {}", e) }),
                        is_error: true,
                    })
                }
            }
        }
        #[cfg(target_os = "linux")]
        {
            match tokio::process::Command::new("notify-send")
                .args([title, body])
                .output()
                .await
            {
                Ok(out) if out.status.success() => {
                    return Ok(ToolResultData {
                        data: json!({ "sent": true, "title": title, "message": "Notification sent successfully." }),
                        is_error: false,
                    })
                }
                Ok(out) => {
                    return Ok(ToolResultData {
                        data: json!({ "sent": false, "error": format!("notify-send failed: {}", String::from_utf8_lossy(&out.stderr)) }),
                        is_error: true,
                    })
                }
                Err(e) => {
                    return Ok(ToolResultData {
                        data: json!({ "sent": false, "error": format!("Failed: {}", e) }),
                        is_error: true,
                    })
                }
            }
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        Ok(ToolResultData {
            data: json!({ "sent": false, "error": "Desktop notifications not supported on this platform" }),
            is_error: true,
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
        }
    }
    #[tokio::test]
    async fn push_notification_missing_title() {
        let r = PushNotificationTool
            .call(
                &json!({ "body": "Hi" }),
                &make_ctx(),
                CancellationToken::new(),
                None,
            )
            .await
            .unwrap();
        assert!(r.is_error);
        assert!(r.data["error"].as_str().unwrap().contains("title"));
    }
    #[tokio::test]
    async fn push_notification_missing_body() {
        let r = PushNotificationTool
            .call(
                &json!({ "title": "T" }),
                &make_ctx(),
                CancellationToken::new(),
                None,
            )
            .await
            .unwrap();
        assert!(r.is_error);
        assert!(r.data["error"].as_str().unwrap().contains("body"));
    }
    #[tokio::test]
    async fn push_notification_properties() {
        assert_eq!(PushNotificationTool.name(), "PushNotification");
        assert!(!PushNotificationTool.is_read_only(&json!({})));
    }
}
