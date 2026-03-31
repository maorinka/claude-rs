use std::path::PathBuf;
use std::time::SystemTime;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

const MAX_RESULTS: usize = 100;

pub struct GlobTool;

#[async_trait]
impl ToolExecutor for GlobTool {
    fn name(&self) -> &str {
        "Glob"
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

        // Build the full glob pattern: <dir>/<pattern>
        let full_pattern = search_dir.join(pattern_str);
        let full_pattern_str = full_pattern.to_string_lossy().to_string();

        // Collect matching paths with their modification times
        let mut entries: Vec<(PathBuf, SystemTime)> = Vec::new();

        match glob::glob(&full_pattern_str) {
            Ok(paths) => {
                for entry in paths {
                    match entry {
                        Ok(path) => {
                            if path.is_file() {
                                let mtime = path
                                    .metadata()
                                    .and_then(|m| m.modified())
                                    .unwrap_or(SystemTime::UNIX_EPOCH);
                                entries.push((path, mtime));
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Glob entry error: {e}");
                        }
                    }
                }
            }
            Err(e) => {
                return Ok(ToolResultData {
                    data: json!({ "error": format!("Invalid glob pattern: {e}") }),
                    is_error: true,
                });
            }
        }

        // Sort by modification time, most recent first
        entries.sort_by(|a, b| b.1.cmp(&a.1));

        let total = entries.len();
        let truncated = total > MAX_RESULTS;
        entries.truncate(MAX_RESULTS);

        // Relativize paths against the search_dir to save tokens
        let filenames: Vec<String> = entries
            .into_iter()
            .map(|(path, _)| {
                path.strip_prefix(&search_dir)
                    .map(|rel| rel.to_string_lossy().to_string())
                    .unwrap_or_else(|_| path.to_string_lossy().to_string())
            })
            .collect();

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
