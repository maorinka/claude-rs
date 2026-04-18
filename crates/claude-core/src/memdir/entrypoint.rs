//! MEMORY.md truncation. Port of `src/memdir/memdir.ts` `truncateEntrypointContent`.

pub const ENTRYPOINT_NAME: &str = "MEMORY.md";
pub const MAX_ENTRYPOINT_LINES: usize = 200;
/// ~125 chars/line at 200 lines. At p97 today; catches long-line indexes
/// that slip past the line cap (p100 observed: 197KB under 200 lines).
pub const MAX_ENTRYPOINT_BYTES: usize = 25_000;

#[derive(Debug, Clone, PartialEq)]
pub struct EntrypointTruncation {
    pub content: String,
    pub line_count: usize,
    pub byte_count: usize,
    pub was_line_truncated: bool,
    pub was_byte_truncated: bool,
}

fn format_file_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{}B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1}KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1}MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// Truncate MEMORY.md content to both the line cap and the byte cap,
/// appending a warning that names which cap fired. Line-truncates first
/// (natural boundary), then byte-truncates at the last newline before the
/// cap so we don't cut mid-line. Mirrors TS `truncateEntrypointContent`.
pub fn truncate_entrypoint_content(raw: &str) -> EntrypointTruncation {
    let trimmed = raw.trim();
    let lines: Vec<&str> = trimmed.split('\n').collect();
    let line_count = lines.len();
    let byte_count = trimmed.len();

    let was_line_truncated = line_count > MAX_ENTRYPOINT_LINES;
    // Check the original byte count — long lines are the failure mode the
    // byte cap targets; post-line-truncation size would understate.
    let was_byte_truncated = byte_count > MAX_ENTRYPOINT_BYTES;

    if !was_line_truncated && !was_byte_truncated {
        return EntrypointTruncation {
            content: trimmed.to_string(),
            line_count,
            byte_count,
            was_line_truncated: false,
            was_byte_truncated: false,
        };
    }

    let mut truncated: String = if was_line_truncated {
        lines[..MAX_ENTRYPOINT_LINES].join("\n")
    } else {
        trimmed.to_string()
    };

    if truncated.len() > MAX_ENTRYPOINT_BYTES {
        let slice = &truncated[..MAX_ENTRYPOINT_BYTES.min(truncated.len())];
        let cut_at = slice.rfind('\n').unwrap_or(MAX_ENTRYPOINT_BYTES.min(truncated.len()));
        truncated.truncate(cut_at);
    }

    let reason = match (was_line_truncated, was_byte_truncated) {
        (false, true) => format!(
            "{} (limit: {}) — index entries are too long",
            format_file_size(byte_count),
            format_file_size(MAX_ENTRYPOINT_BYTES)
        ),
        (true, false) => format!("{} lines (limit: {})", line_count, MAX_ENTRYPOINT_LINES),
        (true, true) => format!(
            "{} lines and {}",
            line_count,
            format_file_size(byte_count)
        ),
        (false, false) => unreachable!("early return above"),
    };

    let content = format!(
        "{truncated}\n\n> WARNING: {ENTRYPOINT_NAME} is {reason}. Only part of it was loaded. \
         Keep index entries to one line under ~200 chars; move detail into topic files.",
    );

    EntrypointTruncation {
        content,
        line_count,
        byte_count,
        was_line_truncated,
        was_byte_truncated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_content_passes_through() {
        let t = truncate_entrypoint_content("one\ntwo\nthree");
        assert!(!t.was_line_truncated);
        assert!(!t.was_byte_truncated);
        assert_eq!(t.content, "one\ntwo\nthree");
    }

    #[test]
    fn line_truncation_fires() {
        let many: String = (0..250).map(|i| format!("line{}", i)).collect::<Vec<_>>().join("\n");
        let t = truncate_entrypoint_content(&many);
        assert!(t.was_line_truncated);
        assert!(t.content.contains("WARNING"));
        assert!(t.content.contains("250 lines"));
    }

    #[test]
    fn byte_truncation_fires() {
        let huge = "x".repeat(MAX_ENTRYPOINT_BYTES + 100);
        let t = truncate_entrypoint_content(&huge);
        assert!(t.was_byte_truncated);
        assert!(t.content.contains("WARNING"));
    }
}
