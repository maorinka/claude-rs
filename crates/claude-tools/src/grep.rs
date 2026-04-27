use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

/// Result of locating a `rg` binary.
pub(crate) enum RgBinary {
    /// A native `rg` binary found at the given path.
    Native(PathBuf),
    /// The Claude multi-call binary that acts as rg when invoked with ARGV0=rg.
    ClaudeMultiCall(PathBuf),
}

/// Locate the `rg` (ripgrep) binary.
///
/// Strategy:
/// 1. Check common hard-coded paths where ripgrep is frequently installed.
/// 2. Check if the Claude multi-call binary is available and supports ripgrep mode.
/// 3. Fall back to just `"rg"` and let the OS resolve it from PATH.
pub(crate) fn find_rg() -> RgBinary {
    // Common installation paths (macOS / Linux)
    let home_rg = std::env::var("HOME")
        .map(|h| format!("{}/.local/bin/rg", h))
        .unwrap_or_default();

    let native_candidates: Vec<&str> = vec![
        "/usr/local/bin/rg",
        "/opt/homebrew/bin/rg",
        "/usr/bin/rg",
        "/bin/rg",
        "/snap/bin/rg",
        &home_rg,
        "/home/linuxbrew/.linuxbrew/bin/rg",
    ];

    for candidate in &native_candidates {
        if candidate.is_empty() {
            continue;
        }
        let p = PathBuf::from(candidate);
        if p.is_file() {
            return RgBinary::Native(p);
        }
    }

    // Check if the Claude multi-call binary exists (it acts as rg when ARGV0=rg).
    // We look for it in the versioned path pattern used by Claude Code.
    if let Ok(home) = std::env::var("HOME") {
        let versions_dir = PathBuf::from(&home).join(".local/share/claude/versions");
        if let Ok(entries) = std::fs::read_dir(&versions_dir) {
            let mut binaries: Vec<PathBuf> = entries
                .flatten()
                .filter(|e| e.path().is_file())
                .map(|e| e.path())
                .collect();
            // Sort descending (latest version first)
            binaries.sort();
            binaries.reverse();
            if let Some(binary) = binaries.first() {
                return RgBinary::ClaudeMultiCall(binary.clone());
            }
        }
    }

    // Last resort: rely on PATH resolution at exec time
    RgBinary::Native(PathBuf::from("rg"))
}

/// VCS directories that ripgrep should exclude.
const VCS_GLOBS: &[&str] = &["!.git", "!.svn", "!.hg", "!.bzr", "!.jj", "!.sl"];

/// Maximum column width passed to ripgrep.
const MAX_COLUMNS: usize = 500;

/// Default number of results to return when head_limit is not specified.
const DEFAULT_HEAD_LIMIT: usize = 250;

/// Maximum total character size of the output.
const MAX_RESULT_SIZE_CHARS: usize = 20_000;

pub struct GrepTool;

#[async_trait]
impl ToolExecutor for GrepTool {
    fn name(&self) -> &str {
        "Grep"
    }

    fn description(&self) -> String {
        r#"A powerful search tool built on ripgrep

  Usage:
  - ALWAYS use Grep for search tasks. NEVER invoke `grep` or `rg` as a Bash command. The Grep tool has been optimized for correct permissions and access.
  - Supports full regex syntax (e.g., "log.*Error", "function\s+\w+")
  - Filter files with glob parameter (e.g., "*.js", "**/*.tsx") or type parameter (e.g., "js", "py", "rust")
  - Output modes: "content" shows matching lines, "files_with_matches" shows only file paths (default), "count" shows match counts
  - Use Agent tool for open-ended searches requiring multiple rounds
  - Pattern syntax: Uses ripgrep (not grep) - literal braces need escaping (use `interface\{\}` to find `interface{}` in Go code)
  - Multiline matching: By default patterns match within single lines only. For cross-line patterns like `struct \{[\s\S]*?field`, use `multiline: true`
"#.to_string()
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    fn is_read_only(&self, _input: &Value) -> bool {
        true
    }

    fn max_result_size_chars(&self) -> usize {
        MAX_RESULT_SIZE_CHARS
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "The regular expression pattern to search for in file contents"
                },
                "path": {
                    "type": "string",
                    "description": "File or directory to search in. Defaults to current working directory."
                },
                "glob": {
                    "type": "string",
                    "description": "Glob pattern to filter files (e.g. \"*.js\", \"*.{ts,tsx}\") - maps to rg --glob"
                },
                "output_mode": {
                    "type": "string",
                    "enum": ["content", "files_with_matches", "count"],
                    "description": "Output mode. Defaults to \"files_with_matches\"."
                },
                "-B": {
                    "type": "number",
                    "description": "Number of lines to show before each match (requires output_mode: content)"
                },
                "-A": {
                    "type": "number",
                    "description": "Number of lines to show after each match (requires output_mode: content)"
                },
                "-C": {
                    "type": "number",
                    "description": "Number of lines to show before and after each match (requires output_mode: content)"
                },
                "-n": {
                    "type": "boolean",
                    "description": "Show line numbers in output"
                },
                "-i": {
                    "type": "boolean",
                    "description": "Case insensitive search"
                },
                "type": {
                    "type": "string",
                    "description": "File type to search (e.g. js, py, rust, go)"
                },
                "head_limit": {
                    "type": "number",
                    "description": "Limit output to first N lines/entries. Defaults to 250."
                },
                "offset": {
                    "type": "number",
                    "description": "Skip first N lines/entries before applying head_limit."
                },
                "multiline": {
                    "type": "boolean",
                    "description": "Enable multiline mode where . matches newlines."
                }
            },
            "required": ["pattern"]
        })
    }

    async fn call(
        &self,
        input: &Value,
        ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let pattern = match input["pattern"].as_str() {
            Some(p) => p.to_string(),
            None => {
                return Ok(ToolResultData {
                    data: json!({ "error": "pattern is required" }),
                    is_error: true,
                });
            }
        };

        let output_mode = input["output_mode"]
            .as_str()
            .unwrap_or("files_with_matches");

        let head_limit = input["head_limit"]
            .as_u64()
            .map(|v| v as usize)
            .unwrap_or(DEFAULT_HEAD_LIMIT);

        let offset = input["offset"].as_u64().map(|v| v as usize).unwrap_or(0);

        // Build the rg command
        let mut cmd = match find_rg() {
            RgBinary::Native(path) => Command::new(path),
            RgBinary::ClaudeMultiCall(path) => {
                #[allow(unused_mut)]
                let mut c = Command::new(&path);
                #[cfg(unix)]
                {
                    #[allow(unused_imports)]
                    use std::os::unix::process::CommandExt as _;
                    c.arg0("rg");
                }
                c
            }
        };

        // Use -e for pattern to handle dash-prefixed patterns safely
        cmd.arg("-e").arg(&pattern);

        // Always search hidden files and directories (.env, .github/, .claude/, etc.)
        // Matches TS GrepTool.ts line 330: `const args = ['--hidden']`
        cmd.arg("--hidden");

        // Output mode flags
        match output_mode {
            "content" => {
                // default rg behavior already outputs content
            }
            "count" => {
                cmd.arg("--count");
            }
            _ => {
                // files_with_matches
                cmd.arg("--files-with-matches");
            }
        }

        // Max columns
        cmd.arg(format!("--max-columns={}", MAX_COLUMNS));

        // VCS exclusions
        for glob in VCS_GLOBS {
            cmd.arg("--glob").arg(glob);
        }

        // Optional flags
        if input["-i"].as_bool().unwrap_or(false) {
            cmd.arg("--ignore-case");
        }

        // -n defaults to true in content mode (mirrors TS default: show_line_numbers = true).
        // Only applies in content mode — TS also gates this on output_mode === 'content'.
        let show_line_numbers = input["-n"].as_bool().unwrap_or(true);
        if show_line_numbers && output_mode == "content" {
            cmd.arg("--line-number");
        }

        if input["multiline"].as_bool().unwrap_or(false) {
            cmd.arg("--multiline");
            cmd.arg("--multiline-dotall");
        }

        // Context lines (only meaningful for content mode)
        if let Some(c) = input["-C"].as_u64() {
            cmd.arg(format!("--context={}", c));
        } else {
            if let Some(b) = input["-B"].as_u64() {
                cmd.arg(format!("--before-context={}", b));
            }
            if let Some(a) = input["-A"].as_u64() {
                cmd.arg(format!("--after-context={}", a));
            }
        }

        // File type filter
        if let Some(t) = input["type"].as_str() {
            cmd.arg("--type").arg(t);
        }

        // Glob filter
        if let Some(g) = input["glob"].as_str() {
            cmd.arg("--glob").arg(g);
        }

        // Search path
        let search_path = if let Some(p) = input["path"].as_str() {
            std::path::PathBuf::from(p)
        } else {
            ctx.working_directory.clone()
        };
        cmd.arg(&search_path);

        // Run rg
        let output = cmd.output().await?;

        // rg exits 0 when matches found, 1 when no matches, 2 on error
        let exit_code = output.status.code().unwrap_or(2);
        if exit_code == 2 {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Ok(ToolResultData {
                data: json!({ "error": stderr }),
                is_error: true,
            });
        }

        let raw = String::from_utf8_lossy(&output.stdout).to_string();

        // Relativize paths: replace absolute search path prefix with relative path
        let working_dir = &ctx.working_directory;
        let relativized = relativize_paths(&raw, &search_path, working_dir);

        // Split into lines, apply offset and head_limit
        let all_lines: Vec<&str> = relativized.lines().collect();
        let total = all_lines.len();

        let sliced: Vec<&str> = all_lines
            .into_iter()
            .skip(offset)
            .take(head_limit)
            .collect();

        let applied_limit = sliced.len();

        match output_mode {
            "content" => {
                let content = sliced.join("\n");
                let content = truncate_to_max(&content, MAX_RESULT_SIZE_CHARS);
                Ok(ToolResultData {
                    data: json!({
                        "mode": "content",
                        "numFiles": count_unique_files_in_content(&content),
                        "filenames": extract_filenames_from_content(&content),
                        "content": content,
                        "numLines": applied_limit,
                        "appliedLimit": applied_limit,
                        "appliedOffset": offset,
                    }),
                    is_error: false,
                })
            }
            "count" => {
                // count mode: each line is "filename:count"
                let content = sliced.join("\n");
                let num_files = sliced.len();
                let filenames: Vec<&str> = sliced
                    .iter()
                    .map(|l| l.split(':').next().unwrap_or(l))
                    .collect();
                Ok(ToolResultData {
                    data: json!({
                        "mode": "count",
                        "numFiles": num_files,
                        "filenames": filenames,
                        "content": content,
                        "numLines": applied_limit,
                        "appliedLimit": applied_limit,
                        "appliedOffset": offset,
                    }),
                    is_error: false,
                })
            }
            _ => {
                // files_with_matches: each line is a file path
                let filenames: Vec<&str> = sliced.iter().map(|l| l.trim()).collect();
                let num_files = filenames.len();
                let mut data = json!({
                    "mode": "files_with_matches",
                    "numFiles": num_files,
                    "filenames": filenames,
                });
                if applied_limit > 0 || offset > 0 {
                    data["appliedLimit"] = json!(applied_limit);
                    data["appliedOffset"] = json!(offset);
                }
                if total > offset + head_limit {
                    data["truncated"] = json!(true);
                }
                Ok(ToolResultData {
                    data,
                    is_error: false,
                })
            }
        }
    }
}

/// Replace absolute path prefix with a relative path so output is portable.
fn relativize_paths(
    raw: &str,
    search_path: &std::path::Path,
    working_dir: &std::path::Path,
) -> String {
    // Compute how to relativize the search_path relative to working_dir
    let rel_prefix: std::path::PathBuf = if search_path.starts_with(working_dir) {
        search_path
            .strip_prefix(working_dir)
            .unwrap_or(search_path)
            .to_path_buf()
    } else {
        search_path.to_path_buf()
    };

    let abs_str = search_path.to_string_lossy();
    let rel_str = rel_prefix.to_string_lossy();

    if abs_str == rel_str {
        // No change needed
        return raw.to_string();
    }

    raw.replace(abs_str.as_ref(), rel_str.as_ref())
}

/// Truncate a string to at most `max_chars` characters.
fn truncate_to_max(s: &str, max_chars: usize) -> String {
    if s.len() <= max_chars {
        s.to_string()
    } else {
        // Try to cut at a line boundary
        let truncated = &s[..max_chars];
        if let Some(pos) = truncated.rfind('\n') {
            truncated[..=pos].to_string()
        } else {
            truncated.to_string()
        }
    }
}

/// Count unique file names appearing in content-mode output.
/// In content mode, rg by default doesn't print file names per line unless
/// --with-filename is used. We return 0 if we cannot determine.
fn count_unique_files_in_content(content: &str) -> usize {
    use std::collections::HashSet;
    let mut files: HashSet<&str> = HashSet::new();
    for line in content.lines() {
        // Lines like "path/to/file.rs:42:match text" or just match text
        if let Some(colon_pos) = line.find(':') {
            let candidate = &line[..colon_pos];
            // Only count if it looks like a path (no spaces, non-empty)
            if !candidate.is_empty() && !candidate.contains(' ') {
                files.insert(candidate);
            }
        }
    }
    files.len()
}

/// Extract unique filenames from content-mode output.
fn extract_filenames_from_content(content: &str) -> Vec<String> {
    use std::collections::HashSet;
    let mut files: HashSet<String> = HashSet::new();
    for line in content.lines() {
        if let Some(colon_pos) = line.find(':') {
            let candidate = &line[..colon_pos];
            if !candidate.is_empty() && !candidate.contains(' ') {
                files.insert(candidate.to_string());
            }
        }
    }
    let mut result: Vec<String> = files.into_iter().collect();
    result.sort();
    result
}
