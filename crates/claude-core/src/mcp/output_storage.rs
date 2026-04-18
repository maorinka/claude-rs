//! MCP output storage helpers.
//!
//! Port of `src/utils/mcpOutputStorage.ts`. Large MCP tool results
//! that shouldn't go through the model's context are persisted to
//! disk; binary responses go with a mime-derived extension so the
//! saved file opens with native tools. This module ports the pure
//! helpers: format-description strings, extension mapping,
//! binary-content classifier, and the large-output instructions text.
//!
//! The `persistBinaryContent` async writer that depends on the tool-
//! result directory abstraction lives in claude-tools; this module
//! ships the sibling helpers all callers share.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpResultType {
    ToolResult,
    StructuredContent,
    ContentArray,
}

/// Human-readable description of an MCP result's shape.
/// Matches TS `getFormatDescription`.
pub fn get_format_description(result_type: McpResultType, schema: Option<&str>) -> String {
    match result_type {
        McpResultType::ToolResult => "Plain text".into(),
        McpResultType::StructuredContent => match schema {
            Some(s) => format!("JSON with schema: {s}"),
            None => "JSON".into(),
        },
        McpResultType::ContentArray => match schema {
            Some(s) => format!("JSON array with schema: {s}"),
            None => "JSON array".into(),
        },
    }
}

/// Build the instruction text Claude receives when a tool result was
/// persisted to disk instead of inlined. Matches TS
/// `getLargeOutputInstructions` verbatim including the sequential-
/// chunks requirement + "explicitly describe what portion you read"
/// clause.
pub fn get_large_output_instructions(
    raw_output_path: &str,
    content_length: usize,
    format_description: &str,
    max_read_length: Option<usize>,
) -> String {
    let base = format!(
        "Error: result ({} characters) exceeds maximum allowed tokens. Output has been saved to {}.\n\
         Format: {}\n\
         Use offset and limit parameters to read specific portions of the file, search within it for specific content, and jq to make structured queries.\n\
         REQUIREMENTS FOR SUMMARIZATION/ANALYSIS/REVIEW:\n\
         - You MUST read the content from the file at {} in sequential chunks until 100% of the content has been read.\n",
        format_with_thousands(content_length),
        raw_output_path,
        format_description,
        raw_output_path,
    );

    let truncation_warning = match max_read_length {
        Some(max) => format!(
            "- If you receive truncation warnings when reading the file (\"[N lines truncated]\"), reduce the chunk size until you have read 100% of the content without truncation ***DO NOT PROCEED UNTIL YOU HAVE DONE THIS***. Bash output is limited to {} chars.\n",
            format_with_thousands(max)
        ),
        None => "- If you receive truncation warnings when reading the file, reduce the chunk size until you have read 100% of the content without truncation.\n".into(),
    };

    let completion = "- Before producing ANY summary or analysis, you MUST explicitly describe what portion of the content you have read. ***If you did not read the entire content, you MUST explicitly state this.***\n";

    format!("{base}{truncation_warning}{completion}")
}

fn format_with_thousands(n: usize) -> String {
    let s = n.to_string();
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    for (i, c) in chars.iter().enumerate() {
        if i > 0 && (chars.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(*c);
    }
    out
}

/// Map a MIME type to a file extension suitable for the persisted
/// filename. Unknown types get `"bin"`. Matches TS
/// `extensionForMimeType` verbatim.
pub fn extension_for_mime_type(mime: Option<&str>) -> &'static str {
    let Some(raw) = mime else {
        return "bin";
    };
    let mt = raw
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    match mt.as_str() {
        "application/pdf" => "pdf",
        "application/json" => "json",
        "text/csv" => "csv",
        "text/plain" => "txt",
        "text/html" => "html",
        "text/markdown" => "md",
        "application/zip" => "zip",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => "docx",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => "xlsx",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => "pptx",
        "application/msword" => "doc",
        "application/vnd.ms-excel" => "xls",
        "audio/mpeg" => "mp3",
        "audio/wav" => "wav",
        "audio/ogg" => "ogg",
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        _ => "bin",
    }
}

/// Heuristic: is this content-type a binary blob that should be saved
/// to disk rather than inlined into the model context?
///
/// Text-ish types (text/\*, application/json, +json, application/xml,
/// +xml, javascript, form-urlencoded) are NOT binary. Everything else
/// is treated as binary.
pub fn is_binary_content_type(content_type: &str) -> bool {
    if content_type.is_empty() {
        return false;
    }
    let mt = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if mt.starts_with("text/") {
        return false;
    }
    if mt.ends_with("+json") || mt == "application/json" {
        return false;
    }
    if mt.ends_with("+xml") || mt == "application/xml" {
        return false;
    }
    if mt.starts_with("application/javascript") {
        return false;
    }
    if mt == "application/x-www-form-urlencoded" {
        return false;
    }
    true
}

/// Format a file size in human-friendly bytes/KB/MB/GB units. Matches
/// TS `formatFileSize` semantics.
pub fn format_file_size(bytes: usize) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b < KB {
        format!("{}B", bytes)
    } else if b < MB {
        format!("{:.1}KB", b / KB)
    } else if b < GB {
        format!("{:.1}MB", b / MB)
    } else {
        format!("{:.1}GB", b / GB)
    }
}

/// Build the message telling Claude where binary content was saved.
/// Matches TS `getBinaryBlobSavedMessage`.
pub fn get_binary_blob_saved_message(
    filepath: &str,
    mime_type: Option<&str>,
    size: usize,
    source_description: &str,
) -> String {
    let mt = mime_type.unwrap_or("unknown type");
    format!(
        "{}Binary content ({}, {}) saved to {}",
        source_description,
        mt,
        format_file_size(size),
        filepath
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_description_matches_ts() {
        assert_eq!(
            get_format_description(McpResultType::ToolResult, None),
            "Plain text"
        );
        assert_eq!(
            get_format_description(McpResultType::StructuredContent, None),
            "JSON"
        );
        assert_eq!(
            get_format_description(McpResultType::StructuredContent, Some("{foo: str}")),
            "JSON with schema: {foo: str}"
        );
        assert_eq!(
            get_format_description(McpResultType::ContentArray, Some("str[]")),
            "JSON array with schema: str[]"
        );
    }

    #[test]
    fn mime_extension_known_types() {
        assert_eq!(extension_for_mime_type(Some("application/pdf")), "pdf");
        assert_eq!(extension_for_mime_type(Some("image/png")), "png");
        assert_eq!(
            extension_for_mime_type(Some("text/plain; charset=utf-8")),
            "txt"
        );
        assert_eq!(extension_for_mime_type(Some("IMAGE/JPEG")), "jpg");
        assert_eq!(extension_for_mime_type(None), "bin");
        assert_eq!(extension_for_mime_type(Some("weird/unknown")), "bin");
    }

    #[test]
    fn binary_classifier_basics() {
        // Text-ish: not binary
        assert!(!is_binary_content_type("text/plain"));
        assert!(!is_binary_content_type("application/json"));
        assert!(!is_binary_content_type("application/xml"));
        assert!(!is_binary_content_type("application/javascript"));
        assert!(!is_binary_content_type("application/vnd.api+json"));
        assert!(!is_binary_content_type("image/svg+xml"));
        assert!(!is_binary_content_type("application/x-www-form-urlencoded"));

        // Empty string is not flagged (matches TS).
        assert!(!is_binary_content_type(""));

        // Binary formats
        assert!(is_binary_content_type("application/pdf"));
        assert!(is_binary_content_type("image/png"));
        assert!(is_binary_content_type(
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        ));
    }

    #[test]
    fn format_file_size_units() {
        assert_eq!(format_file_size(42), "42B");
        assert_eq!(format_file_size(2048), "2.0KB");
        assert_eq!(format_file_size(5 * 1024 * 1024), "5.0MB");
        assert_eq!(format_file_size(3 * 1024 * 1024 * 1024), "3.0GB");
    }

    #[test]
    fn format_thousands_separator() {
        assert_eq!(format_with_thousands(1), "1");
        assert_eq!(format_with_thousands(100), "100");
        assert_eq!(format_with_thousands(1_000), "1,000");
        assert_eq!(format_with_thousands(1_000_000), "1,000,000");
    }

    #[test]
    fn large_output_instructions_mention_path_and_size() {
        let s = get_large_output_instructions(
            "/tmp/out.txt",
            123_456,
            "Plain text",
            Some(8192),
        );
        assert!(s.contains("/tmp/out.txt"));
        assert!(s.contains("123,456 characters"));
        assert!(s.contains("Bash output is limited"));
        assert!(s.contains("8,192 chars"));
    }

    #[test]
    fn binary_saved_message_shape() {
        let m = get_binary_blob_saved_message(
            "/tmp/x.pdf",
            Some("application/pdf"),
            2048,
            "MCP server returned: ",
        );
        assert!(m.starts_with("MCP server returned: "));
        assert!(m.contains("application/pdf"));
        assert!(m.contains("2.0KB"));
        assert!(m.contains("/tmp/x.pdf"));
    }
}
