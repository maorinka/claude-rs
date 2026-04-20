use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

const MAX_OUTPUT_CHARS: usize = 30_000;
const SUPPORTED_LANGUAGES: &[&str] = &["python", "node", "javascript", "ruby"];

pub struct REPLTool;

fn truncate_output(s: String) -> String {
    if s.len() <= MAX_OUTPUT_CHARS {
        s
    } else {
        format!(
            "{}...\n[output truncated at {} chars]",
            &s[..MAX_OUTPUT_CHARS],
            MAX_OUTPUT_CHARS
        )
    }
}

/// Returns (executable, flag) for the given language.
fn language_command(lang: &str) -> Option<(&'static str, &'static str)> {
    match lang {
        "python" => Some(("python3", "-c")),
        "node" | "javascript" => Some(("node", "-e")),
        "ruby" => Some(("ruby", "-e")),
        _ => None,
    }
}

#[async_trait]
impl ToolExecutor for REPLTool {
    fn name(&self) -> &str {
        "REPL"
    }

    fn description(&self) -> String {
        r#"Execute code in a REPL (Read-Eval-Print Loop) for the specified language.

Supported languages:
- python: Executes via `python3 -c`
- node/javascript: Executes via `node -e`
- ruby: Executes via `ruby -e`

Use this for quick code evaluation, testing snippets, or running one-off computations without creating files."#
            .to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "language": {
                    "type": "string",
                    "enum": ["python", "node", "javascript", "ruby"],
                    "description": "The programming language to use."
                },
                "code": {
                    "type": "string",
                    "description": "The code to execute."
                }
            },
            "required": ["language", "code"]
        })
    }

    async fn call(
        &self,
        input: &Value,
        ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let language = match input.get("language").and_then(|v| v.as_str()) {
            Some(l) => l,
            None => {
                return Ok(ToolResultData {
                    data: json!({ "error": "missing required field: language" }),
                    is_error: true,
                });
            }
        };

        let code = match input.get("code").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => {
                return Ok(ToolResultData {
                    data: json!({ "error": "missing required field: code" }),
                    is_error: true,
                });
            }
        };

        let (executable, flag) = match language_command(language) {
            Some(cmd) => cmd,
            None => {
                return Ok(ToolResultData {
                    data: json!({
                        "error": format!(
                            "Unsupported language '{}'. Supported: {}",
                            language,
                            SUPPORTED_LANGUAGES.join(", ")
                        )
                    }),
                    is_error: true,
                });
            }
        };

        if code.trim().is_empty() {
            return Ok(ToolResultData {
                data: json!({ "error": "code must not be empty" }),
                is_error: true,
            });
        }

        let output = tokio::process::Command::new(executable)
            .args([flag, code])
            .current_dir(&ctx.working_directory)
            .output()
            .await;

        match output {
            Ok(output) => {
                let stdout = truncate_output(String::from_utf8_lossy(&output.stdout).to_string());
                let stderr = truncate_output(String::from_utf8_lossy(&output.stderr).to_string());
                let exit_code = output.status.code().unwrap_or(-1);

                Ok(ToolResultData {
                    data: json!({
                        "language": language,
                        "stdout": stdout,
                        "stderr": stderr,
                        "exitCode": exit_code
                    }),
                    is_error: exit_code != 0,
                })
            }
            Err(e) => Ok(ToolResultData {
                data: json!({
                    "error": format!(
                        "Failed to execute '{}': {}. Make sure {} is installed and in PATH.",
                        executable, e, executable
                    )
                }),
                is_error: true,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ReadFileState;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn make_ctx() -> ToolUseContext {
        ToolUseContext::for_test(PathBuf::from("/tmp"), Arc::new(std::sync::Mutex::new(ReadFileState::new())), crate::registry::PermissionMode::Default)
    }

    #[tokio::test]
    async fn repl_missing_language() {
        let tool = REPLTool;
        let input = json!({ "code": "print('hello')" });
        let ctx = make_ctx();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(result.is_error);
        assert!(result.data["error"]
            .as_str()
            .unwrap()
            .contains("missing required field: language"));
    }

    #[tokio::test]
    async fn repl_missing_code() {
        let tool = REPLTool;
        let input = json!({ "language": "python" });
        let ctx = make_ctx();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(result.is_error);
        assert!(result.data["error"]
            .as_str()
            .unwrap()
            .contains("missing required field: code"));
    }

    #[tokio::test]
    async fn repl_unsupported_language() {
        let tool = REPLTool;
        let input = json!({ "language": "cobol", "code": "DISPLAY 'HELLO'" });
        let ctx = make_ctx();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(result.is_error);
        assert!(result.data["error"]
            .as_str()
            .unwrap()
            .contains("Unsupported language"));
    }

    #[tokio::test]
    async fn repl_empty_code() {
        let tool = REPLTool;
        let input = json!({ "language": "python", "code": "  " });
        let ctx = make_ctx();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(result.is_error);
        assert!(result.data["error"]
            .as_str()
            .unwrap()
            .contains("must not be empty"));
    }

    #[tokio::test]
    async fn repl_python_execution() {
        let tool = REPLTool;
        let input = json!({ "language": "python", "code": "print('hello from python')" });
        let ctx = make_ctx();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        // This test may fail if python3 is not installed
        if !result.is_error {
            assert!(result.data["stdout"]
                .as_str()
                .unwrap()
                .contains("hello from python"));
            assert_eq!(result.data["exitCode"].as_i64().unwrap(), 0);
        }
    }

    #[tokio::test]
    async fn repl_python_error() {
        let tool = REPLTool;
        let input = json!({ "language": "python", "code": "raise ValueError('test error')" });
        let ctx = make_ctx();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        // This test may fail if python3 is not installed, but if it runs:
        if result.data.get("exitCode").is_some() {
            assert!(result.is_error);
            assert_ne!(result.data["exitCode"].as_i64().unwrap(), 0);
        }
    }

    #[test]
    fn language_command_mapping() {
        assert_eq!(language_command("python"), Some(("python3", "-c")));
        assert_eq!(language_command("node"), Some(("node", "-e")));
        assert_eq!(language_command("javascript"), Some(("node", "-e")));
        assert_eq!(language_command("ruby"), Some(("ruby", "-e")));
        assert_eq!(language_command("cobol"), None);
    }
}
