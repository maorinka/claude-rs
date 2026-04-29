use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::diff_utils::structured_patch_for_display;
use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use crate::tool_path::expand_tool_path;
use crate::write::FILE_HISTORY;
use claude_core::types::events::ToolResultData;

/// Maximum number of characters shown in error message snippets.
const MAX_DISPLAY_LEN: usize = 100;

/// Truncate a string to at most `MAX_DISPLAY_LEN` characters for display in error messages.
fn truncate_display(s: &str) -> String {
    if s.len() <= MAX_DISPLAY_LEN {
        s.to_string()
    } else {
        format!("{}…", &s[..MAX_DISPLAY_LEN])
    }
}

/// Detected line-ending flavour for a file buffer. `Crlf` when the
/// CRLF count beats the LF count; `Lf` otherwise. Ports TS
/// `LineEndingType` + `detectLineEndingsForString` at
/// `fileRead.ts:18,51`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEndings {
    Lf,
    Crlf,
}

/// Maximum sample size for line-ending detection, measured in
/// UTF-16 code units to match TS `raw.slice(0, 4096)` at
/// `fileRead.ts:92`. JS strings index by UTF-16 code unit, so
/// capping on UTF-8 bytes would diverge on content that mixes
/// many 2-byte UTF-8 chars (Cyrillic, most Asian scripts) near
/// the boundary: TS's 4096-unit window reaches further into the
/// byte stream than a 4096-byte cap would.
const LINE_ENDING_SAMPLE_UTF16_UNITS: u32 = 4096;

/// Walk `content` scalar-by-scalar, accumulating UTF-16 code-unit
/// positions; stop once the *starting* position of the next char
/// reaches `LINE_ENDING_SAMPLE_UTF16_UNITS`. Tally `\n` as LF, or
/// CRLF when preceded by `\r`. Byte-for-byte port of TS
/// `detectLineEndingsForString` applied to the sliced prefix —
/// since `\r` and `\n` are BMP single-unit scalars, a TS slice
/// that cuts mid-surrogate-pair never exposes a lone surrogate
/// that could be mistaken for a line terminator, so walking by
/// Rust scalars gives identical counts. Ties go to `Lf`
/// (`crlfCount > lfCount`).
pub fn detect_line_endings(content: &str) -> LineEndings {
    let mut crlf = 0usize;
    let mut lf = 0usize;
    let mut prev_is_cr = false;
    let mut pos_u16: u32 = 0;
    for c in content.chars() {
        if pos_u16 >= LINE_ENDING_SAMPLE_UTF16_UNITS {
            break;
        }
        if c == '\n' {
            if prev_is_cr {
                crlf += 1;
            } else {
                lf += 1;
            }
        }
        prev_is_cr = c == '\r';
        pos_u16 += c.len_utf16() as u32;
    }
    if crlf > lf {
        LineEndings::Crlf
    } else {
        LineEndings::Lf
    }
}

/// Strip CRLF → LF without touching lone LFs that were already
/// present. TS's normalisation step in `FileEditTool.ts:214` uses
/// `replaceAll('\r\n', '\n')` which is equivalent to this.
pub fn normalize_to_lf(content: &str) -> String {
    content.replace("\r\n", "\n")
}

fn error_result(msg: impl Into<String>) -> ToolResultData {
    ToolResultData {
        data: json!({ "error": msg.into() }),
        is_error: true,
    }
}

pub struct FileEditTool;

#[async_trait]
impl ToolExecutor for FileEditTool {
    fn name(&self) -> &str {
        "Edit"
    }

    fn description(&self) -> String {
        r#"Performs exact string replacements in files.

Usage:
- You must use your `Read` tool at least once in the conversation before editing. This tool will error if you attempt an edit without reading the file.
- When editing text from Read tool output, ensure you preserve the exact indentation (tabs/spaces) as it appears AFTER the line number prefix. The line number prefix format is: line number + tab. Everything after that is the actual file content to match. Never include any part of the line number prefix in the old_string or new_string.
- ALWAYS prefer editing existing files in the codebase. NEVER write new files unless explicitly required.
- Only use emojis if the user explicitly requests it. Avoid adding emojis to files unless asked.
- The edit will FAIL if `old_string` is not unique in the file. Either provide a larger string with more surrounding context to make it unique or use `replace_all` to change every instance of `old_string`.
- Use `replace_all` for replacing and renaming strings across the file. This parameter is useful if you want to rename a variable for instance."#.to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the file to edit"
                },
                "old_string": {
                    "type": "string",
                    "description": "The string to search for and replace"
                },
                "new_string": {
                    "type": "string",
                    "description": "The string to replace old_string with"
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "If true, replace all occurrences; otherwise require exactly one match",
                    "default": false
                }
            },
            "required": ["file_path", "old_string", "new_string"]
        })
    }

    async fn call(
        &self,
        input: &Value,
        ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let file_path = match input["file_path"].as_str() {
            Some(p) => p,
            None => return Ok(error_result("Missing required field: file_path")),
        };
        let file_path = expand_tool_path(file_path, &ctx.working_directory);
        let file_path = file_path.as_str();
        let old_string = match input["old_string"].as_str() {
            Some(s) => s,
            None => return Ok(error_result("Missing required field: old_string")),
        };
        let new_string = match input["new_string"].as_str() {
            Some(s) => s,
            None => return Ok(error_result("Missing required field: new_string")),
        };
        let replace_all = input["replace_all"].as_bool().unwrap_or(false);

        let path = std::path::Path::new(file_path);

        // Guard: refuse to raw-edit Jupyter notebook files. Raw string replacement
        // inside notebook JSON can corrupt cell metadata, output arrays, and the
        // nbformat schema. Mirrors TS FileEditTool lines 266-273.
        if path.extension().is_some_and(|ext| ext == "ipynb") {
            return Ok(error_result(
                "File is a Jupyter Notebook (.ipynb). Use the NotebookEdit tool to edit \
                 notebook cells instead. Raw string replacement in notebook JSON can \
                 corrupt cell metadata and break the notebook format.",
            ));
        }

        // Team-memory secret guard — fires BEFORE the new-file-creation
        // branch so a brand-new team-memory file with a secret never lands
        // on disk. Matches TS `FileEditTool.validateInput` which scans
        // `new_string` (not the projected post-edit buffer) at
        // FileEditTool.ts:144. cwd comes from the request-scoped tool
        // context, mirroring TS's AsyncLocalStorage `getCwd()`.
        if let Some(msg) = claude_core::teams::team_mem_secret_guard::check_team_mem_secrets(
            path,
            new_string,
            &ctx.working_directory,
        ) {
            return Ok(error_result(msg));
        }

        // If old_string is non-empty and the file doesn't exist -> error
        if !path.exists() {
            if old_string.is_empty() {
                // Creating a new file with no old content to replace -- write empty->new
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(path, new_string)?;
                return Ok(ToolResultData {
                    data: json!({
                        "filePath": file_path,
                        "oldString": old_string,
                        "newString": new_string,
                        "originalFile": "",
                        "structuredPatch": structured_patch_for_display("", new_string),
                        "userModified": false,
                        "replaceAll": replace_all
                    }),
                    is_error: false,
                });
            }
            return Ok(error_result(format!("File not found: {}", file_path)));
        }

        // Staleness check: ensure the file has been read and not modified since.
        if let Err(msg) = crate::write::check_file_staleness(file_path, path, &ctx.read_file_state)
        {
            return Ok(error_result(msg));
        }

        // Take a snapshot before editing.
        if let Ok(mut tracker) = FILE_HISTORY.lock() {
            let _ = tracker.snapshot(path);
        }

        let raw_original = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => return Ok(error_result(format!("Failed to read file: {}", e))),
        };

        // CRLF preservation: detect original line endings from the raw
        // file bytes, then work against the LF-normalised form so an
        // `old_string` with LF endings (the model's default) matches
        // content that may have CRLF on disk. On write, re-normalise
        // LF→CRLF if the original file was CRLF-dominant. Matches TS
        // `FileEditTool.ts:212-214` (normalize on read) + `:491`
        // (writeTextContent re-normalizes on write).
        let endings = detect_line_endings(&raw_original);
        let original = normalize_to_lf(&raw_original);

        // Count occurrences (against LF-normalised content — matches TS
        // which operates on the normalised buffer).
        let count = original.matches(old_string).count();

        if count == 0 {
            return Ok(error_result(format!(
                "String not found in file.\nSearched for: {}",
                truncate_display(old_string)
            )));
        }

        if count > 1 && !replace_all {
            return Ok(error_result(format!(
                "Found {} occurrences of the search string but replace_all is false. \
                 Use replace_all=true to replace all occurrences, or provide a more specific \
                 old_string that matches exactly once.\nSearched for: {}",
                count,
                truncate_display(old_string)
            )));
        }

        let new_content = if replace_all {
            original.replace(old_string, new_string)
        } else {
            // replace first occurrence only
            original.replacen(old_string, new_string, 1)
        };

        // Re-normalize to CRLF on write when the original file used CRLF.
        // Strip any existing CRLF first so a `new_string` that itself
        // contains CRLF (raw model output, or copy-pasted Windows content)
        // doesn't become CRCRLF after the join. Matches TS
        // `writeTextContent` at file.ts:90-94.
        let to_write = match endings {
            LineEndings::Crlf => new_content.replace("\r\n", "\n").replace('\n', "\r\n"),
            LineEndings::Lf => new_content.clone(),
        };

        // Ensure parent directories exist (in case of a new path -- defensive)
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent)?;
            }
        }

        if let Err(e) = std::fs::write(path, &to_write) {
            return Ok(error_result(format!("Failed to write file: {}", e)));
        }

        // Update read state after successful edit. Store the
        // LF-normalised post-edit content (not the CRLF-re-encoded
        // disk form) so the next staleness check's content
        // comparison uses the same normalisation as
        // `check_file_staleness`. Mirrors TS `FileEditTool.ts:520-525`.
        if let Ok(mut state) = ctx.read_file_state.lock() {
            state.update_after_write(file_path, Some(new_content.clone()));
        }

        Ok(ToolResultData {
            data: json!({
                "filePath": file_path,
                "oldString": old_string,
                "newString": new_string,
                "originalFile": original, // LF-normalised form; TS parity
                "structuredPatch": structured_patch_for_display(&original, &new_content),
                "userModified": false,
                "replaceAll": replace_all
            }),
            is_error: false,
        })
    }

    fn is_destructive(&self, _input: &Value) -> bool {
        true
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }

    fn is_read_only(&self, _input: &Value) -> bool {
        false
    }

    fn check_permissions(
        &self,
        input: &Value,
        context: &claude_core::permissions::ToolPermissionContext,
    ) -> claude_core::permissions::PermissionResult {
        let Some(file_path) = input["file_path"].as_str() else {
            return claude_core::permissions::PermissionResult::passthrough("");
        };
        let file_path = expand_tool_path(file_path, &context.working_directory);
        match claude_core::permissions::check_write_permission_for_tool(&file_path, context) {
            claude_core::permissions::PermissionDecision::Allow(allow) => {
                claude_core::permissions::PermissionResult::Allow(allow)
            }
            claude_core::permissions::PermissionDecision::Ask(ask) => {
                claude_core::permissions::PermissionResult::Ask(ask)
            }
            claude_core::permissions::PermissionDecision::Deny(deny) => {
                claude_core::permissions::PermissionResult::Deny(deny)
            }
        }
    }

    fn max_result_size_chars(&self) -> usize {
        100_000
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_endings_empty_is_lf() {
        assert_eq!(detect_line_endings(""), LineEndings::Lf);
    }

    #[test]
    fn detect_endings_lone_cr_is_lf() {
        // TS `detectLineEndingsForString` only tallies on `\n`; lone
        // CR (old-Mac) never increments either counter.
        assert_eq!(detect_line_endings("a\rb"), LineEndings::Lf);
        assert_eq!(detect_line_endings("\r"), LineEndings::Lf);
    }

    #[test]
    fn detect_endings_single_crlf_wins() {
        assert_eq!(detect_line_endings("a\r\n"), LineEndings::Crlf);
    }

    #[test]
    fn detect_endings_single_lf_wins() {
        assert_eq!(detect_line_endings("a\n"), LineEndings::Lf);
    }

    #[test]
    fn detect_endings_tie_goes_to_lf() {
        // TS uses strict `crlfCount > lfCount` — 1 > 1 is false.
        assert_eq!(detect_line_endings("\r\n\n"), LineEndings::Lf);
    }

    #[test]
    fn detect_endings_caps_at_4096_utf16_units() {
        // First 4096 code units are pure LF; anything beyond should
        // not be counted. TS samples `raw.slice(0, 4096)` at
        // fileRead.ts:92.
        let mut s = String::with_capacity(8192);
        for _ in 0..4000 {
            s.push_str("x\n"); // ASCII: 1 byte = 1 UTF-16 unit
        }
        // Append enough CRLF to dominate if we scanned the whole buffer.
        for _ in 0..4000 {
            s.push_str("x\r\n");
        }
        // With the cap: LFs in first 4096 units dominate, so Lf.
        assert_eq!(detect_line_endings(&s), LineEndings::Lf);
    }

    #[test]
    fn detect_endings_cap_is_utf16_not_bytes() {
        // Regression for codex CR finding: byte-capping at 4096
        // would miss `\r\n` that TS's UTF-16 code-unit slice still
        // includes. Build content where a CRLF majority falls
        // between byte 4096 and UTF-16 unit 4096 — i.e. after the
        // byte-cap but before the code-unit-cap. 2-byte UTF-8 chars
        // (U+00E4 'ä': 2 bytes = 1 UTF-16 unit) double the byte
        // stride relative to the code-unit stride.
        let mut s = String::new();
        // 5 bare LFs so the baseline favours Lf (5 > any CRLF count
        // that a byte-cap would see, which is 0).
        for _ in 0..5 {
            s.push('\n');
        }
        // Pad with 'ä' until we're ~4090 bytes / ~2045 units in
        // (stay inside a byte-cap's window so the LFs alone would
        // still tip it to Lf with no CRLFs).
        for _ in 0..2045 {
            s.push('ä'); // 2 bytes, 1 UTF-16 unit
        }
        // Now place 6 CRLFs, each separated by an 'ä'. First CRLF
        // starts at byte ~4095 / unit ~2050 — outside the byte cap
        // (4096) but well inside the code-unit cap (4096).
        for _ in 0..6 {
            s.push('ä');
            s.push_str("\r\n");
        }
        // TS (code-unit cap): sees all 6 CRLFs + 5 LFs → Crlf.
        // Byte-cap (old Rust): sees 0 CRLFs + 5 LFs → Lf (wrong).
        assert_eq!(
            detect_line_endings(&s),
            LineEndings::Crlf,
            "UTF-16 code-unit cap must match TS `raw.slice(0, 4096)`; \
             byte-capping would miss CRLFs beyond byte 4096"
        );
    }

    #[test]
    fn normalize_to_lf_strips_only_crlf() {
        assert_eq!(normalize_to_lf("a\r\nb"), "a\nb");
        // Lone CR preserved.
        assert_eq!(normalize_to_lf("a\rb"), "a\rb");
        // Lone LF untouched.
        assert_eq!(normalize_to_lf("a\nb"), "a\nb");
    }
}
