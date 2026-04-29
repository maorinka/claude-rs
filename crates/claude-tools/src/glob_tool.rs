use std::path::{Path, PathBuf};

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use crate::grep::{find_rg, RgBinary};
use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

const MAX_RESULTS: usize = 100;

pub struct GlobTool;

#[async_trait]
impl ToolExecutor for GlobTool {
    fn name(&self) -> &str {
        "Glob"
    }

    fn description(&self) -> String {
        r#"- Fast file pattern matching tool that works with any codebase size
- Supports glob patterns like "**/*.js" or "src/**/*.ts"
- Returns matching file paths sorted by modification time
- Use this tool when you need to find files by name patterns
- When you are doing an open ended search that may require multiple rounds of globbing and grepping, use the Agent tool instead"#.to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern to match files (e.g. '**/*.rs', '*.md')"
                },
                "path": {
                    "type": "string",
                    "description": "Directory to search in. Defaults to the working directory."
                }
            },
            "required": ["pattern"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    fn is_read_only(&self, _input: &Value) -> bool {
        true
    }

    fn to_auto_classifier_input(&self, input: &Value) -> Option<String> {
        Some(input["pattern"].as_str().unwrap_or_default().to_string())
    }

    fn max_result_size_chars(&self) -> usize {
        100_000
    }

    async fn call(
        &self,
        input: &Value,
        ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let start = std::time::Instant::now();

        let pattern_str = match input["pattern"].as_str() {
            Some(p) => p,
            None => {
                return Ok(ToolResultData {
                    data: json!({"error": "Missing required field: pattern"}),
                    is_error: true,
                });
            }
        };

        // Determine the search directory
        let search_dir: PathBuf = if let Some(path) = input["path"].as_str() {
            PathBuf::from(path)
        } else {
            ctx.working_directory.clone()
        };

        // Validate that search_dir exists and is a directory
        if !search_dir.exists() {
            return Ok(ToolResultData {
                data: json!({
                    "error": format!("Path does not exist: {}", search_dir.display())
                }),
                is_error: true,
            });
        }
        if !search_dir.is_dir() {
            return Ok(ToolResultData {
                data: json!({
                    "error": format!("Path is not a directory: {}", search_dir.display())
                }),
                is_error: true,
            });
        }

        let mut cmd = ripgrep_files_command();
        cmd.current_dir(&search_dir)
            .arg("--files")
            .arg("--glob")
            .arg(pattern_str)
            .arg("--sort=modified")
            .arg("--no-ignore")
            .arg("--hidden");

        let output = cmd.output().await?;
        if !output.status.success() && output.status.code() != Some(1) {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Ok(ToolResultData {
                data: json!({ "error": if stderr.is_empty() { "Glob search failed".to_string() } else { stderr } }),
                is_error: true,
            });
        }

        let all_filenames: Vec<String> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|line| normalize_glob_result_path(line, &search_dir))
            .collect();

        let total = all_filenames.len();
        let truncated = total > MAX_RESULTS;
        let filenames: Vec<String> = all_filenames.into_iter().take(MAX_RESULTS).collect();

        let num_files = filenames.len() as u32;
        let duration_ms = start.elapsed().as_millis() as u64;

        Ok(ToolResultData {
            data: json!({
                "filenames": filenames,
                "durationMs": duration_ms,
                "numFiles": num_files,
                "truncated": truncated,
            }),
            is_error: false,
        })
    }
}

fn ripgrep_files_command() -> Command {
    match find_rg() {
        RgBinary::Native(path) => Command::new(path),
        RgBinary::ClaudeMultiCall(path) => {
            #[allow(unused_mut)]
            let mut cmd = Command::new(&path);
            #[cfg(unix)]
            {
                #[allow(unused_imports)]
                use std::os::unix::process::CommandExt as _;
                cmd.arg0("rg");
            }
            cmd
        }
    }
}

fn normalize_glob_result_path(line: &str, search_dir: &Path) -> String {
    let path = Path::new(line);
    let value = if path.is_absolute() {
        path.strip_prefix(search_dir).unwrap_or(path)
    } else {
        path
    };
    value.to_string_lossy().to_string()
}
