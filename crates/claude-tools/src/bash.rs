use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::io::AsyncReadExt;
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

const MAX_OUTPUT_CHARS: usize = 30_000;

pub struct BashTool;

fn truncate(s: String) -> String {
    if s.len() <= MAX_OUTPUT_CHARS {
        s
    } else {
        s[..MAX_OUTPUT_CHARS].to_string()
    }
}

#[async_trait]
impl ToolExecutor for BashTool {
    fn name(&self) -> &str {
        "Bash"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The bash command to execute"
                },
                "description": {
                    "type": "string",
                    "description": "Optional description of what the command does"
                },
                "run_in_background": {
                    "type": "boolean",
                    "description": "Whether to run the command in the background"
                }
            },
            "required": ["command"]
        })
    }

    async fn call(
        &self,
        input: &Value,
        ctx: &ToolUseContext,
        cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let command = input["command"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing 'command' field"))?
            .to_string();

        // If already cancelled before we even start, return immediately
        if cancel.is_cancelled() {
            return Ok(ToolResultData {
                data: json!({
                    "stdout": "",
                    "stderr": "",
                    "code": -1,
                    "interrupted": true
                }),
                is_error: false,
            });
        }

        let mut child = tokio::process::Command::new("bash")
            .arg("-c")
            .arg(&command)
            .current_dir(&ctx.working_directory)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;

        // Take pipes before any select so we can read them after wait
        let mut stdout_pipe = child.stdout.take().expect("stdout pipe");
        let mut stderr_pipe = child.stderr.take().expect("stderr pipe");

        tokio::select! {
            _ = cancel.cancelled() => {
                // Kill the child process on cancellation
                let _ = child.kill().await;
                let _ = child.wait().await;
                Ok(ToolResultData {
                    data: json!({
                        "stdout": "",
                        "stderr": "",
                        "code": -1,
                        "interrupted": true
                    }),
                    is_error: false,
                })
            }
            status = child.wait() => {
                let exit_status = status?;
                // Read remaining output after process has exited
                let mut stdout_bytes = Vec::new();
                let mut stderr_bytes = Vec::new();
                let _ = stdout_pipe.read_to_end(&mut stdout_bytes).await;
                let _ = stderr_pipe.read_to_end(&mut stderr_bytes).await;

                let stdout = truncate(String::from_utf8_lossy(&stdout_bytes).into_owned());
                let stderr = truncate(String::from_utf8_lossy(&stderr_bytes).into_owned());
                let code = exit_status.code().unwrap_or(-1);
                Ok(ToolResultData {
                    data: json!({
                        "stdout": stdout,
                        "stderr": stderr,
                        "code": code,
                        "interrupted": false
                    }),
                    is_error: false,
                })
            }
        }
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }

    fn is_read_only(&self, _input: &Value) -> bool {
        false
    }

    fn max_result_size_chars(&self) -> usize {
        30_000
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_short_string() {
        let s = "hello".to_string();
        assert_eq!(truncate(s), "hello");
    }

    #[test]
    fn test_truncate_exactly_at_limit() {
        let s = "a".repeat(MAX_OUTPUT_CHARS);
        assert_eq!(truncate(s.clone()).len(), MAX_OUTPUT_CHARS);
    }

    #[test]
    fn test_truncate_multibyte_at_boundary() {
        // Create a string of ASCII chars up to near the limit, then add multi-byte chars
        // Each emoji (e.g. U+1F600) is 4 bytes in UTF-8
        let padding_len = MAX_OUTPUT_CHARS - 2; // 2 bytes short of limit
        let mut s = "a".repeat(padding_len);
        // Add a 4-byte emoji - bytes [padding_len..padding_len+4]
        // The boundary MAX_OUTPUT_CHARS falls at padding_len+2, which is mid-character
        s.push('\u{1F600}'); // grinning face emoji, 4 bytes
        s.push_str("more text after");

        // This should NOT panic - the buggy version would panic slicing mid-UTF8
        let result = truncate(s);
        // Result should be truncated to before the emoji since the boundary falls inside it
        assert!(result.len() <= MAX_OUTPUT_CHARS);
        assert!(result.is_char_boundary(result.len()));
    }

    #[test]
    fn test_truncate_multibyte_string_of_emojis() {
        // String entirely of 4-byte emoji characters
        // MAX_OUTPUT_CHARS = 30000, each emoji is 4 bytes
        // 30000 / 4 = 7500 emojis fit exactly, but let's go over
        let emoji_count = MAX_OUTPUT_CHARS / 4 + 100;
        let s: String = std::iter::repeat('\u{1F600}').take(emoji_count).collect();
        assert!(s.len() > MAX_OUTPUT_CHARS);

        let result = truncate(s);
        assert!(result.len() <= MAX_OUTPUT_CHARS);
        // Should be valid UTF-8 (no panic on further operations)
        assert!(result.is_char_boundary(result.len()));
    }

    #[test]
    fn test_truncate_two_byte_chars_at_boundary() {
        // Use 2-byte characters (e.g. U+00E9 = 'é', 2 bytes in UTF-8)
        let padding_len = MAX_OUTPUT_CHARS - 1;
        let mut s = "a".repeat(padding_len);
        s.push('é'); // 2 bytes - byte boundary falls in the middle
        s.push_str("extra");

        let result = truncate(s);
        assert!(result.len() <= MAX_OUTPUT_CHARS);
        assert!(result.is_char_boundary(result.len()));
    }

    #[test]
    fn test_truncate_three_byte_cjk_at_boundary() {
        // CJK character U+4E16 ('世') is 3 bytes
        let padding_len = MAX_OUTPUT_CHARS - 1;
        let mut s = "a".repeat(padding_len);
        s.push('世'); // 3 bytes, boundary falls inside
        s.push_str("extra");

        let result = truncate(s);
        assert!(result.len() <= MAX_OUTPUT_CHARS);
        assert!(result.is_char_boundary(result.len()));
    }
}
