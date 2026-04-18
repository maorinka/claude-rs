use anyhow::Result;
use async_trait::async_trait;
use base64::Engine;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

/// Paths that must never be opened (infinite / blocking / sensitive device files).
const BLOCKED_PATHS: &[&str] = &[
    "/dev/zero",
    "/dev/random",
    "/dev/urandom",
    "/dev/stdin",
    "/dev/tty",
    "/dev/null",
];

const DEFAULT_LINE_LIMIT: u64 = 2000;

/// Verbatim port of TS FileReadTool/prompt.ts
/// `renderPromptTemplate(...)` with the default runtime flags
/// (line-format on, targeted-offset suggested, PDF supported).
pub const FILE_READ_PROMPT: &str = include_str!("prompts/file_read.md");

/// Verbatim port of TS FileReadTool/prompt.ts `FILE_UNCHANGED_STUB`.
/// Returned from the dedup path when a file hasn't changed since the
/// last Read in the conversation — the stub points the model back at
/// the prior tool_result so the context isn't duplicated.
pub const FILE_UNCHANGED_STUB: &str =
    "File unchanged since last read. The content from the earlier Read tool_result in this conversation is still current — refer to that instead of re-reading.";

/// TS `MAX_LINES_TO_READ` constant used by the prompt builder.
/// Rust's runtime uses `DEFAULT_LINE_LIMIT` instead, but we expose
/// this so call sites that need the TS literal stay consistent.
pub const MAX_LINES_TO_READ: u64 = 2000;

/// Maximum file size for reading: 256 KB for text files (matches TS MAX_OUTPUT_SIZE).
const MAX_TEXT_FILE_SIZE: u64 = 256 * 1024;

/// Maximum raw PDF size accepted: 20 MB (base64 encoded stays under 32 MB API limit).
const MAX_PDF_RAW_SIZE: u64 = 20 * 1024 * 1024;

/// Maximum pages that can be read per PDF request.
const PDF_MAX_PAGES_PER_READ: u32 = 20;

/// Number of bytes to inspect for binary content detection.
const BINARY_CHECK_SIZE: usize = 8192;

/// Image extensions that can be read and returned as base64 image blocks.
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp"];

/// Binary file extensions that cannot be meaningfully read as text.
/// PDF and image extensions are excluded from this check since FileReadTool renders them natively.
const BINARY_EXTENSIONS: &[&str] = &[
    // Images (non-API-supported)
    ".bmp", ".ico", ".tiff", ".tif", // Videos
    ".mp4", ".mov", ".avi", ".mkv", ".webm", ".wmv", ".flv", ".m4v", ".mpeg", ".mpg",
    // Audio
    ".mp3", ".wav", ".ogg", ".flac", ".aac", ".m4a", ".wma", ".aiff", ".opus",
    // Archives
    ".zip", ".tar", ".gz", ".bz2", ".7z", ".rar", ".xz", ".z", ".tgz", ".iso",
    // Executables/binaries
    ".exe", ".dll", ".so", ".dylib", ".bin", ".o", ".a", ".obj", ".lib", ".app", ".msi", ".deb",
    ".rpm", // Documents (non-PDF)
    ".doc", ".docx", ".xls", ".xlsx", ".ppt", ".pptx", ".odt", ".ods", ".odp", // Fonts
    ".ttf", ".otf", ".woff", ".woff2", ".eot", // Bytecode / VM artifacts
    ".pyc", ".pyo", ".class", ".jar", ".war", ".ear", ".node", ".wasm", ".rlib",
    // Database files
    ".sqlite", ".sqlite3", ".db", ".mdb", ".idx", // Design / 3D
    ".psd", ".ai", ".eps", ".sketch", ".fig", ".xd", ".blend", ".3ds", ".max", // Flash
    ".swf", ".fla", // Lock/profiling data
    ".lockb", ".dat", ".data",
];

pub struct FileReadTool;

/// Returns the lowercase file extension without the leading dot, or empty string.
fn get_extension(path: &str) -> String {
    std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase()
}

/// Check if a file extension indicates an image the API can accept.
fn is_image_extension(ext: &str) -> bool {
    IMAGE_EXTENSIONS.contains(&ext)
}

/// Check if a file path has a known binary extension.
/// PDF and image extensions are excluded (FileReadTool renders them natively).
fn has_binary_extension(path: &str) -> bool {
    let lower = path.to_lowercase();
    BINARY_EXTENSIONS.iter().any(|ext| lower.ends_with(ext))
}

/// Check if binary content is detected in a buffer by looking for null bytes
/// or a high proportion of non-printable characters in the first 8 KB.
fn is_binary_content(buf: &[u8]) -> bool {
    let check_size = buf.len().min(BINARY_CHECK_SIZE);
    if check_size == 0 {
        return false;
    }

    let mut non_printable: usize = 0;
    for &byte in &buf[..check_size] {
        // Null byte is a strong indicator of binary.
        if byte == 0 {
            return true;
        }
        // Count non-printable, non-whitespace bytes.
        // Printable ASCII is 32-126, plus common whitespace (9=tab, 10=LF, 13=CR).
        if byte < 32 && byte != 9 && byte != 10 && byte != 13 {
            non_printable += 1;
        }
    }

    // If more than 10% non-printable, likely binary.
    (non_printable as f64 / check_size as f64) > 0.1
}

/// Map an image extension to the MIME media type string.
fn image_media_type(ext: &str) -> &'static str {
    match ext {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    }
}

/// Parse a PDF page range string. Supported formats:
/// - "5"     -> (5, 5)
/// - "1-10"  -> (1, 10)
/// - "3-"    -> (3, u32::MAX)   [open-ended]
///
/// Returns `None` on invalid input (non-numeric, zero, inverted range).
/// Pages are 1-indexed.
fn parse_pdf_page_range(pages: &str) -> Option<(u32, u32)> {
    let trimmed = pages.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Open-ended range: "N-"
    if trimmed.ends_with('-') {
        let first: u32 = trimmed[..trimmed.len() - 1].parse().ok()?;
        if first < 1 {
            return None;
        }
        return Some((first, u32::MAX));
    }

    if let Some(dash_idx) = trimmed.find('-') {
        // Range: "1-10"
        let first: u32 = trimmed[..dash_idx].parse().ok()?;
        let last: u32 = trimmed[dash_idx + 1..].parse().ok()?;
        if first < 1 || last < 1 || last < first {
            return None;
        }
        Some((first, last))
    } else {
        // Single page: "5"
        let page: u32 = trimmed.parse().ok()?;
        if page < 1 {
            return None;
        }
        Some((page, page))
    }
}

/// Render a Jupyter notebook (.ipynb) file into a structured JSON value
/// containing the cells with their types, source, and outputs.
fn render_notebook(raw: &[u8], file_path: &str) -> Result<ToolResultData> {
    let content = std::str::from_utf8(raw)
        .map_err(|e| anyhow::anyhow!("notebook is not valid UTF-8: {}", e))?;

    let notebook: Value = serde_json::from_str(content)
        .map_err(|e| anyhow::anyhow!("invalid notebook JSON: {}", e))?;

    let language = notebook
        .pointer("/metadata/language_info/name")
        .and_then(|v| v.as_str())
        .unwrap_or("python");

    let cells_raw = match notebook.get("cells").and_then(|c| c.as_array()) {
        Some(arr) => arr,
        None => {
            return Ok(error_result("notebook has no cells array"));
        }
    };

    let mut cells = Vec::new();
    for (i, cell) in cells_raw.iter().enumerate() {
        let cell_type = cell
            .get("cell_type")
            .and_then(|v| v.as_str())
            .unwrap_or("raw");

        let cell_id = cell
            .get("id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("cell-{}", i));

        // Join source lines if it's an array, or take the string directly.
        let source = match cell.get("source") {
            Some(Value::Array(arr)) => arr
                .iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(""),
            Some(Value::String(s)) => s.clone(),
            _ => String::new(),
        };

        let mut cell_json = json!({
            "cell_type": cell_type,
            "cell_id": cell_id,
            "source": source,
        });

        if cell_type == "code" {
            cell_json["language"] = json!(language);

            if let Some(ec) = cell.get("execution_count").and_then(|v| v.as_u64()) {
                cell_json["execution_count"] = json!(ec);
            }

            // Process outputs.
            if let Some(outputs_arr) = cell.get("outputs").and_then(|o| o.as_array()) {
                let mut processed_outputs = Vec::new();
                for output in outputs_arr {
                    let output_type = output
                        .get("output_type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");

                    match output_type {
                        "stream" => {
                            let text = join_text_field(output.get("text"));
                            processed_outputs.push(json!({
                                "output_type": output_type,
                                "text": text,
                            }));
                        }
                        "execute_result" | "display_data" => {
                            let text = output
                                .pointer("/data/text/plain")
                                .map(|v| join_text_field(Some(v)))
                                .unwrap_or_default();
                            processed_outputs.push(json!({
                                "output_type": output_type,
                                "text": text,
                            }));
                        }
                        "error" => {
                            let ename = output.get("ename").and_then(|v| v.as_str()).unwrap_or("");
                            let evalue =
                                output.get("evalue").and_then(|v| v.as_str()).unwrap_or("");
                            let traceback = output
                                .get("traceback")
                                .and_then(|v| v.as_array())
                                .map(|arr| {
                                    arr.iter()
                                        .filter_map(|v| v.as_str())
                                        .collect::<Vec<_>>()
                                        .join("\n")
                                })
                                .unwrap_or_default();
                            processed_outputs.push(json!({
                                "output_type": "error",
                                "text": format!("{}: {}\n{}", ename, evalue, traceback),
                            }));
                        }
                        _ => {}
                    }
                }
                if !processed_outputs.is_empty() {
                    cell_json["outputs"] = json!(processed_outputs);
                }
            }
        }

        cells.push(cell_json);
    }

    let result_data = json!({
        "type": "notebook",
        "file": {
            "filePath": file_path,
            "cells": cells
        }
    });

    Ok(ToolResultData {
        data: result_data,
        is_error: false,
    })
}

/// Helper: join a text field that may be a string or array of strings.
fn join_text_field(val: Option<&Value>) -> String {
    match val {
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join(""),
        Some(Value::String(s)) => s.clone(),
        _ => String::new(),
    }
}

#[async_trait]
impl ToolExecutor for FileReadTool {
    fn name(&self) -> &str {
        "Read"
    }

    fn description(&self) -> String {
        FILE_READ_PROMPT.to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the file to read."
                },
                "offset": {
                    "type": "integer",
                    "description": "0-based line index to start reading from."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of lines to return."
                },
                "pages": {
                    "type": "string",
                    "description": "Page range for PDF files (e.g. \"1-5\")."
                }
            },
            "required": ["file_path"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    fn is_read_only(&self, _input: &Value) -> bool {
        true
    }

    fn max_result_size_chars(&self) -> usize {
        usize::MAX
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
            None => {
                return Ok(error_result("missing required parameter: file_path"));
            }
        };

        // Block dangerous device paths.
        if BLOCKED_PATHS.contains(&file_path) {
            return Ok(error_result(&format!(
                "access to '{}' is blocked for safety reasons",
                file_path
            )));
        }

        let ext = get_extension(file_path);

        // --- Binary extension check (excluding images and PDFs which are handled natively) ---
        if has_binary_extension(file_path) && ext != "pdf" && !is_image_extension(&ext) {
            return Ok(error_result(&format!(
                "This tool cannot read binary files. The file appears to be a binary .{} file. \
                 Please use appropriate tools for binary file analysis.",
                ext
            )));
        }

        // --- Validate pages parameter early (pure string parsing, no I/O) ---
        let pages_param = input["pages"].as_str();
        if let Some(pages) = pages_param {
            match parse_pdf_page_range(pages) {
                None => {
                    return Ok(error_result(&format!(
                        "Invalid pages parameter: \"{}\". Use formats like \"1-5\", \"3\", or \"10-20\". Pages are 1-indexed.",
                        pages
                    )));
                }
                Some((first, last)) => {
                    let range_size = if last == u32::MAX {
                        PDF_MAX_PAGES_PER_READ + 1
                    } else {
                        last - first + 1
                    };
                    if range_size > PDF_MAX_PAGES_PER_READ {
                        return Ok(error_result(&format!(
                            "Page range \"{}\" exceeds maximum of {} pages per request. Please use a smaller range.",
                            pages, PDF_MAX_PAGES_PER_READ
                        )));
                    }
                }
            }
        }

        // --- Image handling ---
        if is_image_extension(&ext) {
            return self.read_image(file_path, &ext).await;
        }

        // --- PDF handling ---
        if ext == "pdf" {
            return self.read_pdf(file_path, pages_param).await;
        }

        // --- Notebook handling ---
        if ext == "ipynb" {
            return self.read_notebook(file_path).await;
        }

        // --- Text file reading ---
        self.read_text_file(file_path, input, ctx).await
    }
}

impl FileReadTool {
    /// Read an image file and return base64-encoded data with metadata.
    async fn read_image(&self, file_path: &str, ext: &str) -> Result<ToolResultData> {
        let raw = match tokio::fs::read(file_path).await {
            Ok(bytes) => bytes,
            Err(e) => {
                return Ok(error_result(&format!("cannot read '{}': {}", file_path, e)));
            }
        };

        let original_size = raw.len();
        let media_type = image_media_type(ext);
        let b64 = base64::engine::general_purpose::STANDARD.encode(&raw);

        let result_data = json!({
            "type": "image",
            "file": {
                "base64": b64,
                "type": media_type,
                "originalSize": original_size,
                "dimensions": null
            }
        });

        Ok(ToolResultData {
            data: result_data,
            is_error: false,
        })
    }

    /// Read a PDF file and return base64-encoded document block.
    async fn read_pdf(&self, file_path: &str, _pages: Option<&str>) -> Result<ToolResultData> {
        let raw = match tokio::fs::read(file_path).await {
            Ok(bytes) => bytes,
            Err(e) => {
                return Ok(error_result(&format!("cannot read '{}': {}", file_path, e)));
            }
        };

        let original_size = raw.len() as u64;

        if original_size > MAX_PDF_RAW_SIZE {
            return Ok(error_result(&format!(
                "PDF file is too large ({} bytes). Maximum supported size is {} bytes ({} MB). \
                 Use the pages parameter to read specific page ranges.",
                original_size,
                MAX_PDF_RAW_SIZE,
                MAX_PDF_RAW_SIZE / (1024 * 1024),
            )));
        }

        let b64 = base64::engine::general_purpose::STANDARD.encode(&raw);

        let result_data = json!({
            "type": "pdf",
            "file": {
                "filePath": file_path,
                "base64": b64,
                "originalSize": original_size
            }
        });

        Ok(ToolResultData {
            data: result_data,
            is_error: false,
        })
    }

    /// Read a Jupyter notebook file and return structured cell data.
    async fn read_notebook(&self, file_path: &str) -> Result<ToolResultData> {
        let raw = match tokio::fs::read(file_path).await {
            Ok(bytes) => bytes,
            Err(e) => {
                return Ok(error_result(&format!("cannot read '{}': {}", file_path, e)));
            }
        };

        render_notebook(&raw, file_path)
    }

    /// Read a text file with offset/limit and binary detection.
    async fn read_text_file(
        &self,
        file_path: &str,
        input: &Value,
        ctx: &ToolUseContext,
    ) -> Result<ToolResultData> {
        // Track whether offset/limit were explicitly supplied by the caller.
        // is_partial should only be true when the user explicitly bounded the read.
        // Mirrors TS: FileReadTool stores `offset`/`limit` as undefined when not provided,
        // and isPartialView is only set by auto-injection paths, not normal reads.
        let explicit_offset = input.get("offset").and_then(|v| v.as_u64()).is_some();
        let explicit_limit = input.get("limit").and_then(|v| v.as_u64()).is_some();
        let offset = input["offset"].as_u64().unwrap_or(0) as usize;
        let limit = input["limit"].as_u64().unwrap_or(DEFAULT_LINE_LIMIT) as usize;

        // Read the file as raw bytes first for binary detection and size check.
        let raw_bytes = match tokio::fs::read(file_path).await {
            Ok(bytes) => bytes,
            Err(e) => {
                return Ok(error_result(&format!("cannot read '{}': {}", file_path, e)));
            }
        };

        let file_size = raw_bytes.len() as u64;

        // File size limit check.
        if file_size > MAX_TEXT_FILE_SIZE {
            return Ok(error_result(&format!(
                "File size ({} bytes, {:.1} KB) exceeds maximum allowed size ({} bytes, {} KB). \
                 Use offset and limit parameters to read specific portions of the file, \
                 or search for specific content instead of reading the whole file.",
                file_size,
                file_size as f64 / 1024.0,
                MAX_TEXT_FILE_SIZE,
                MAX_TEXT_FILE_SIZE / 1024,
            )));
        }

        // Binary content detection.
        if is_binary_content(&raw_bytes) {
            return Ok(error_result(&format!(
                "File '{}' appears to contain binary content and cannot be displayed as text. \
                 Use appropriate tools for binary file analysis.",
                file_path
            )));
        }

        // Convert to string.
        let raw = match std::str::from_utf8(&raw_bytes) {
            Ok(s) => s.to_string(),
            Err(_) => {
                return Ok(error_result(&format!(
                    "File '{}' contains invalid UTF-8 and cannot be displayed as text.",
                    file_path
                )));
            }
        };

        // Record this read in the shared state for staleness tracking.
        // is_partial is only true when the caller explicitly provided offset or limit;
        // a default read (no args) is never partial, even if the file has fewer lines
        // than DEFAULT_LINE_LIMIT. Mirrors TS: isPartialView is only set for bounded reads.
        let is_partial = explicit_offset || explicit_limit;
        if let Ok(mut state) = ctx.read_file_state.lock() {
            // Pass file content for full reads so write/edit can do content-comparison
            // fallback when mtime changes (antivirus/cloud-sync harmless touch).
            let stored_content = if is_partial { None } else { Some(raw.clone()) };
            state.record_read(file_path, is_partial, stored_content);
        }

        // Split into lines.
        let all_lines: Vec<&str> = raw.lines().collect();
        let total_lines = all_lines.len();

        // Apply offset and limit.
        let start = offset.min(total_lines);
        let end = (start + limit).min(total_lines);
        let selected = &all_lines[start..end];

        // Format in cat -n style: "{1-based-line-num}\t{content}"
        let start_line = start + 1; // convert to 1-based
        let mut formatted = String::new();
        for (i, line) in selected.iter().enumerate() {
            let line_num = start_line + i;
            formatted.push_str(&format!("{}\t{}\n", line_num, line));
        }

        let result_data = json!({
            "type": "text",
            "file": {
                "filePath": file_path,
                "content": formatted,
                "numLines": selected.len(),
                "startLine": start_line,
                "totalLines": total_lines
            }
        });

        Ok(ToolResultData {
            data: result_data,
            is_error: false,
        })
    }
}

fn error_result(msg: &str) -> ToolResultData {
    ToolResultData {
        data: json!({ "error": msg }),
        is_error: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ReadFileState;
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tempfile::NamedTempFile;

    // ---- Helper ----

    fn make_temp_file(ext: &str, content: &[u8]) -> NamedTempFile {
        let mut f = tempfile::Builder::new()
            .suffix(&format!(".{}", ext))
            .tempfile()
            .unwrap();
        f.write_all(content).unwrap();
        f.flush().unwrap();
        f
    }

    // ---- Unit tests for helpers ----

    #[test]
    fn test_get_extension() {
        assert_eq!(get_extension("/foo/bar.PNG"), "png");
        assert_eq!(get_extension("/foo/bar.tar.gz"), "gz");
        assert_eq!(get_extension("noext"), "");
    }

    #[test]
    fn test_is_image_extension() {
        assert!(is_image_extension("png"));
        assert!(is_image_extension("jpg"));
        assert!(is_image_extension("jpeg"));
        assert!(is_image_extension("gif"));
        assert!(is_image_extension("webp"));
        assert!(!is_image_extension("bmp"));
        assert!(!is_image_extension("txt"));
    }

    #[test]
    fn test_has_binary_extension() {
        assert!(has_binary_extension("/foo/bar.zip"));
        assert!(has_binary_extension("/foo/bar.exe"));
        assert!(has_binary_extension("/foo/bar.MP3")); // case-insensitive
        assert!(!has_binary_extension("/foo/bar.txt"));
        assert!(!has_binary_extension("/foo/bar.rs"));
        // PDF and image extensions should NOT be flagged as binary
        // because they are handled natively.
        assert!(!has_binary_extension("/foo/bar.png"));
        assert!(!has_binary_extension("/foo/bar.jpg"));
        assert!(!has_binary_extension("/foo/bar.pdf"));
    }

    #[test]
    fn test_binary_content_detection_null_byte() {
        let mut buf = vec![0u8; 100];
        buf[50] = 0; // null byte
        assert!(is_binary_content(&buf));
    }

    #[test]
    fn test_binary_content_detection_high_non_printable() {
        // Create a buffer with >10% non-printable bytes (control chars 1-8, 11, 12, 14-31).
        let mut buf = vec![b'A'; 100];
        for b in buf.iter_mut().take(15) {
            *b = 1; // SOH, a non-printable control character
        }
        assert!(is_binary_content(&buf));
    }

    #[test]
    fn test_binary_content_detection_normal_text() {
        let text = b"Hello, world!\nThis is a normal text file.\n\tWith tabs.\n";
        assert!(!is_binary_content(text));
    }

    #[test]
    fn test_binary_content_detection_empty() {
        assert!(!is_binary_content(&[]));
    }

    #[test]
    fn test_parse_pdf_page_range_single() {
        assert_eq!(parse_pdf_page_range("5"), Some((5, 5)));
    }

    #[test]
    fn test_parse_pdf_page_range_range() {
        assert_eq!(parse_pdf_page_range("1-10"), Some((1, 10)));
    }

    #[test]
    fn test_parse_pdf_page_range_open_ended() {
        assert_eq!(parse_pdf_page_range("3-"), Some((3, u32::MAX)));
    }

    #[test]
    fn test_parse_pdf_page_range_invalid() {
        assert_eq!(parse_pdf_page_range(""), None);
        assert_eq!(parse_pdf_page_range("0"), None);
        assert_eq!(parse_pdf_page_range("abc"), None);
        assert_eq!(parse_pdf_page_range("10-5"), None); // inverted
    }

    #[test]
    fn test_image_media_type() {
        assert_eq!(image_media_type("png"), "image/png");
        assert_eq!(image_media_type("jpg"), "image/jpeg");
        assert_eq!(image_media_type("jpeg"), "image/jpeg");
        assert_eq!(image_media_type("gif"), "image/gif");
        assert_eq!(image_media_type("webp"), "image/webp");
    }

    // ---- Integration tests (async, using the tool) ----

    fn make_tool_context() -> ToolUseContext {
        ToolUseContext {
            working_directory: PathBuf::from("/tmp"),
            read_file_state: Arc::new(std::sync::Mutex::new(ReadFileState::new())),
            permission_mode: crate::registry::PermissionMode::Default,
        }
    }

    #[tokio::test]
    async fn test_read_png_returns_base64_image() {
        // Minimal valid 1x1 PNG (67 bytes).
        let png_data: Vec<u8> = vec![
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // PNG signature
            0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, // IHDR chunk
            0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, // 1x1
            0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49,
            0x44, 0x41, 0x54, // IDAT chunk
            0x08, 0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00, 0x00, 0x00, 0x02, 0x00, 0x01, 0xE2, 0x21,
            0xBC, 0x33, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, // IEND chunk
            0xAE, 0x42, 0x60, 0x82,
        ];
        let f = make_temp_file("png", &png_data);
        let path = f.path().to_str().unwrap();

        let tool = FileReadTool;
        let input = json!({ "file_path": path });
        let ctx = make_tool_context();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(!result.is_error);
        assert_eq!(result.data["type"], "image");
        assert_eq!(result.data["file"]["type"], "image/png");
        assert_eq!(result.data["file"]["originalSize"], png_data.len());

        // Verify the base64 round-trips.
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(result.data["file"]["base64"].as_str().unwrap())
            .unwrap();
        assert_eq!(decoded, png_data);
    }

    #[tokio::test]
    async fn test_read_jpg_returns_base64_image() {
        let jpg_data = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10]; // JFIF header stub
        let f = make_temp_file("jpg", &jpg_data);
        let path = f.path().to_str().unwrap();

        let tool = FileReadTool;
        let input = json!({ "file_path": path });
        let ctx = make_tool_context();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(!result.is_error);
        assert_eq!(result.data["type"], "image");
        assert_eq!(result.data["file"]["type"], "image/jpeg");
    }

    #[tokio::test]
    async fn test_read_gif_returns_base64_image() {
        let gif_data = b"GIF89a".to_vec();
        let f = make_temp_file("gif", &gif_data);
        let path = f.path().to_str().unwrap();

        let tool = FileReadTool;
        let input = json!({ "file_path": path });
        let ctx = make_tool_context();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(!result.is_error);
        assert_eq!(result.data["type"], "image");
        assert_eq!(result.data["file"]["type"], "image/gif");
    }

    #[tokio::test]
    async fn test_read_webp_returns_base64_image() {
        let webp_data = b"RIFF\x00\x00\x00\x00WEBP".to_vec();
        let f = make_temp_file("webp", &webp_data);
        let path = f.path().to_str().unwrap();

        let tool = FileReadTool;
        let input = json!({ "file_path": path });
        let ctx = make_tool_context();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(!result.is_error);
        assert_eq!(result.data["type"], "image");
        assert_eq!(result.data["file"]["type"], "image/webp");
    }

    #[tokio::test]
    async fn test_read_pdf_returns_document_block() {
        // A minimal PDF stub (not a valid PDF, but sufficient for base64 encoding).
        let pdf_data = b"%PDF-1.4 minimal stub".to_vec();
        let f = make_temp_file("pdf", &pdf_data);
        let path = f.path().to_str().unwrap();

        let tool = FileReadTool;
        let input = json!({ "file_path": path });
        let ctx = make_tool_context();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(!result.is_error);
        assert_eq!(result.data["type"], "pdf");
        assert_eq!(result.data["file"]["filePath"], path);
        assert_eq!(result.data["file"]["originalSize"], pdf_data.len());
        // Verify base64 round-trips.
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(result.data["file"]["base64"].as_str().unwrap())
            .unwrap();
        assert_eq!(decoded, pdf_data);
    }

    #[tokio::test]
    async fn test_read_ipynb_returns_notebook_cells() {
        let notebook_json = r##"{
            "metadata": {
                "language_info": { "name": "python" }
            },
            "cells": [
                {
                    "cell_type": "markdown",
                    "id": "md1",
                    "source": ["# Hello\n", "World"]
                },
                {
                    "cell_type": "code",
                    "id": "code1",
                    "source": "print('hello')",
                    "execution_count": 1,
                    "outputs": [
                        {
                            "output_type": "stream",
                            "text": "hello\n"
                        }
                    ]
                },
                {
                    "cell_type": "code",
                    "id": "code2",
                    "source": ["1/0"],
                    "execution_count": 2,
                    "outputs": [
                        {
                            "output_type": "error",
                            "ename": "ZeroDivisionError",
                            "evalue": "division by zero",
                            "traceback": ["line 1", "line 2"]
                        }
                    ]
                }
            ]
        }"##;
        let f = make_temp_file("ipynb", notebook_json.as_bytes());
        let path = f.path().to_str().unwrap();

        let tool = FileReadTool;
        let input = json!({ "file_path": path });
        let ctx = make_tool_context();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(!result.is_error);
        assert_eq!(result.data["type"], "notebook");
        assert_eq!(result.data["file"]["filePath"], path);

        let cells = result.data["file"]["cells"].as_array().unwrap();
        assert_eq!(cells.len(), 3);

        // First cell: markdown
        assert_eq!(cells[0]["cell_type"], "markdown");
        assert_eq!(cells[0]["cell_id"], "md1");
        assert_eq!(cells[0]["source"], "# Hello\nWorld");

        // Second cell: code with stream output
        assert_eq!(cells[1]["cell_type"], "code");
        assert_eq!(cells[1]["language"], "python");
        assert_eq!(cells[1]["execution_count"], 1);
        let outputs1 = cells[1]["outputs"].as_array().unwrap();
        assert_eq!(outputs1[0]["output_type"], "stream");
        assert_eq!(outputs1[0]["text"], "hello\n");

        // Third cell: code with error output
        assert_eq!(cells[2]["cell_type"], "code");
        let outputs2 = cells[2]["outputs"].as_array().unwrap();
        assert_eq!(outputs2[0]["output_type"], "error");
        assert!(outputs2[0]["text"]
            .as_str()
            .unwrap()
            .contains("ZeroDivisionError"));
    }

    #[tokio::test]
    async fn test_binary_file_detection_null_bytes() {
        // Create a file with null bytes embedded in text.
        let mut data = b"Hello".to_vec();
        data.push(0x00); // null byte
        data.extend_from_slice(b"World");
        let f = make_temp_file("dat2", &data); // .dat2 is not in binary extensions
        let path = f.path().to_str().unwrap();

        let tool = FileReadTool;
        let input = json!({ "file_path": path });
        let ctx = make_tool_context();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(result.is_error);
        assert!(result.data["error"]
            .as_str()
            .unwrap()
            .contains("binary content"));
    }

    #[tokio::test]
    async fn test_binary_extension_blocked() {
        let f = make_temp_file("zip", b"PK\x03\x04");
        let path = f.path().to_str().unwrap();

        let tool = FileReadTool;
        let input = json!({ "file_path": path });
        let ctx = make_tool_context();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(result.is_error);
        assert!(result.data["error"]
            .as_str()
            .unwrap()
            .contains("cannot read binary files"));
    }

    #[tokio::test]
    async fn test_file_size_limit_exceeded() {
        // Create a file larger than MAX_TEXT_FILE_SIZE (256 KB).
        let big = vec![b'A'; (MAX_TEXT_FILE_SIZE + 1) as usize];
        let f = make_temp_file("txt", &big);
        let path = f.path().to_str().unwrap();

        let tool = FileReadTool;
        let input = json!({ "file_path": path });
        let ctx = make_tool_context();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(result.is_error);
        assert!(result.data["error"]
            .as_str()
            .unwrap()
            .contains("exceeds maximum allowed size"));
    }

    #[tokio::test]
    async fn test_pdf_size_limit_exceeded() {
        // Create a PDF larger than MAX_PDF_RAW_SIZE (20 MB).
        let mut big = b"%PDF-1.4 ".to_vec();
        big.resize((MAX_PDF_RAW_SIZE + 1) as usize, b'X');
        let f = make_temp_file("pdf", &big);
        let path = f.path().to_str().unwrap();

        let tool = FileReadTool;
        let input = json!({ "file_path": path });
        let ctx = make_tool_context();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(result.is_error);
        assert!(result.data["error"].as_str().unwrap().contains("too large"));
    }

    #[tokio::test]
    async fn test_pdf_page_range_validation() {
        let f = make_temp_file("pdf", b"%PDF-1.4");
        let path = f.path().to_str().unwrap();

        let tool = FileReadTool;
        let ctx = make_tool_context();
        let cancel = CancellationToken::new();

        // Invalid page range
        let input = json!({ "file_path": path, "pages": "abc" });
        let result = tool.call(&input, &ctx, cancel.clone(), None).await.unwrap();
        assert!(result.is_error);
        assert!(result.data["error"]
            .as_str()
            .unwrap()
            .contains("Invalid pages parameter"));

        // Too many pages
        let input = json!({ "file_path": path, "pages": "1-25" });
        let result = tool.call(&input, &ctx, cancel.clone(), None).await.unwrap();
        assert!(result.is_error);
        assert!(result.data["error"]
            .as_str()
            .unwrap()
            .contains("exceeds maximum"));
    }

    #[tokio::test]
    async fn test_read_normal_text_file() {
        let content = "line one\nline two\nline three\n";
        let f = make_temp_file("txt", content.as_bytes());
        let path = f.path().to_str().unwrap();

        let tool = FileReadTool;
        let input = json!({ "file_path": path });
        let ctx = make_tool_context();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(!result.is_error);
        assert_eq!(result.data["type"], "text");
        assert_eq!(result.data["file"]["totalLines"], 3);
        assert_eq!(result.data["file"]["numLines"], 3);
        assert_eq!(result.data["file"]["startLine"], 1);
        let content_out = result.data["file"]["content"].as_str().unwrap();
        assert!(content_out.contains("1\tline one"));
        assert!(content_out.contains("2\tline two"));
        assert!(content_out.contains("3\tline three"));
    }

    #[tokio::test]
    async fn test_read_text_file_with_offset_and_limit() {
        let content = "a\nb\nc\nd\ne\n";
        let f = make_temp_file("txt", content.as_bytes());
        let path = f.path().to_str().unwrap();

        let tool = FileReadTool;
        let input = json!({ "file_path": path, "offset": 2, "limit": 2 });
        let ctx = make_tool_context();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(!result.is_error);
        assert_eq!(result.data["file"]["numLines"], 2);
        assert_eq!(result.data["file"]["startLine"], 3); // 1-based: offset 2 -> line 3
        let content_out = result.data["file"]["content"].as_str().unwrap();
        assert!(content_out.contains("3\tc\n"));
        assert!(content_out.contains("4\td\n"));
    }

    #[tokio::test]
    async fn test_blocked_device_path() {
        let tool = FileReadTool;
        let input = json!({ "file_path": "/dev/zero" });
        let ctx = make_tool_context();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(result.is_error);
        assert!(result.data["error"].as_str().unwrap().contains("blocked"));
    }

    #[tokio::test]
    async fn test_file_not_found() {
        let tool = FileReadTool;
        let input = json!({ "file_path": "/tmp/nonexistent_file_12345.txt" });
        let ctx = make_tool_context();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(result.is_error);
        assert!(result.data["error"]
            .as_str()
            .unwrap()
            .contains("cannot read"));
    }

    #[tokio::test]
    async fn test_notebook_auto_generated_cell_ids() {
        let notebook_json = r##"{
            "metadata": { "language_info": { "name": "r" } },
            "cells": [
                { "cell_type": "code", "source": "1+1", "outputs": [] },
                { "cell_type": "markdown", "source": "# Note" }
            ]
        }"##;
        let f = make_temp_file("ipynb", notebook_json.as_bytes());
        let path = f.path().to_str().unwrap();

        let tool = FileReadTool;
        let input = json!({ "file_path": path });
        let ctx = make_tool_context();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(!result.is_error);
        let cells = result.data["file"]["cells"].as_array().unwrap();
        assert_eq!(cells[0]["cell_id"], "cell-0");
        assert_eq!(cells[0]["language"], "r");
        assert_eq!(cells[1]["cell_id"], "cell-1");
        // Markdown cells should not have a language field.
        assert!(cells[1].get("language").is_none());
    }
}
