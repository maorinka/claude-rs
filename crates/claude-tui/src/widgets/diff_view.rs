use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

/// A parsed line from a unified diff.
#[derive(Clone, Debug, PartialEq)]
pub enum DiffLineKind {
    /// Context line (unchanged)
    Context,
    /// Added line
    Added,
    /// Removed line
    Removed,
    /// Hunk header (@@...@@)
    HunkHeader,
    /// File header (--- or +++)
    FileHeader,
}

/// A single line from a parsed diff.
#[derive(Clone, Debug)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub content: String,
    /// Old file line number (None for added lines / headers)
    pub old_lineno: Option<u32>,
    /// New file line number (None for removed lines / headers)
    pub new_lineno: Option<u32>,
}

/// Parse a unified diff string into structured DiffLines.
pub fn parse_unified_diff(diff_text: &str) -> Vec<DiffLine> {
    let mut lines = Vec::new();
    let mut old_line: u32 = 0;
    let mut new_line: u32 = 0;

    for raw_line in diff_text.lines() {
        if raw_line.starts_with("@@") {
            // Parse hunk header: @@ -old_start,old_count +new_start,new_count @@
            if let Some((old_start, new_start)) = parse_hunk_header(raw_line) {
                old_line = old_start;
                new_line = new_start;
            }
            lines.push(DiffLine {
                kind: DiffLineKind::HunkHeader,
                content: raw_line.to_string(),
                old_lineno: None,
                new_lineno: None,
            });
        } else if raw_line.starts_with("---") || raw_line.starts_with("+++") {
            lines.push(DiffLine {
                kind: DiffLineKind::FileHeader,
                content: raw_line.to_string(),
                old_lineno: None,
                new_lineno: None,
            });
        } else if let Some(content) = raw_line.strip_prefix('+') {
            lines.push(DiffLine {
                kind: DiffLineKind::Added,
                content: content.to_string(),
                old_lineno: None,
                new_lineno: Some(new_line),
            });
            new_line += 1;
        } else if let Some(content) = raw_line.strip_prefix('-') {
            lines.push(DiffLine {
                kind: DiffLineKind::Removed,
                content: content.to_string(),
                old_lineno: Some(old_line),
                new_lineno: None,
            });
            old_line += 1;
        } else {
            // Context line (may start with ' ' or be plain)
            let content = raw_line.strip_prefix(' ').unwrap_or(raw_line);
            lines.push(DiffLine {
                kind: DiffLineKind::Context,
                content: content.to_string(),
                old_lineno: Some(old_line),
                new_lineno: Some(new_line),
            });
            old_line += 1;
            new_line += 1;
        }
    }

    lines
}

/// Parse a hunk header line to extract start line numbers.
/// Format: @@ -old_start[,old_count] +new_start[,new_count] @@
fn parse_hunk_header(line: &str) -> Option<(u32, u32)> {
    let trimmed = line.trim_start_matches('@').trim();
    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }

    let old_start = parts[0]
        .trim_start_matches('-')
        .split(',')
        .next()?
        .parse::<u32>()
        .ok()?;

    let new_start = parts[1]
        .trim_start_matches('+')
        .split(',')
        .next()?
        .parse::<u32>()
        .ok()?;

    Some((old_start, new_start))
}

/// Ratatui widget that renders a parsed diff with colored lines
/// and dual line numbers.
pub struct DiffViewWidget {
    lines: Vec<DiffLine>,
    scroll_offset: usize,
}

impl DiffViewWidget {
    pub fn new(diff_text: &str) -> Self {
        Self {
            lines: parse_unified_diff(diff_text),
            scroll_offset: 0,
        }
    }

    pub fn from_lines(lines: Vec<DiffLine>) -> Self {
        Self {
            lines,
            scroll_offset: 0,
        }
    }

    #[allow(dead_code)]
    pub fn with_scroll(mut self, offset: usize) -> Self {
        self.scroll_offset = offset;
        self
    }

    /// Render a single DiffLine into a styled ratatui Line.
    fn render_diff_line(diff_line: &DiffLine, width: u16) -> Line<'static> {
        let gutter_width = 10; // "NNNN NNNN " = 10 chars

        let (marker, line_style) = match diff_line.kind {
            DiffLineKind::Added => ("+", Style::default().fg(Color::Green)),
            DiffLineKind::Removed => ("-", Style::default().fg(Color::Red)),
            DiffLineKind::Context => (" ", Style::default().fg(Color::White)),
            DiffLineKind::HunkHeader => {
                return Line::from(Span::styled(
                    diff_line.content.clone(),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ));
            }
            DiffLineKind::FileHeader => {
                return Line::from(Span::styled(
                    diff_line.content.clone(),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ));
            }
        };

        let old_str = diff_line
            .old_lineno
            .map(|n| format!("{:>4}", n))
            .unwrap_or_else(|| "    ".to_string());
        let new_str = diff_line
            .new_lineno
            .map(|n| format!("{:>4}", n))
            .unwrap_or_else(|| "    ".to_string());

        let content_width = (width as usize).saturating_sub(gutter_width + 1); // +1 for marker
        let content = if diff_line.content.chars().count() > content_width {
            truncate_with_ellipsis(&diff_line.content, content_width)
        } else {
            diff_line.content.clone()
        };

        Line::from(vec![
            Span::styled(
                format!("{} {} ", old_str, new_str),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(marker.to_string(), line_style),
            Span::styled(content, line_style),
        ])
    }
}

impl Widget for DiffViewWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || self.lines.is_empty() {
            return;
        }

        let visible_height = area.height as usize;
        let start = self.scroll_offset.min(self.lines.len().saturating_sub(1));
        let end = (start + visible_height).min(self.lines.len());

        for (i, diff_line) in self.lines[start..end].iter().enumerate() {
            let y = area.y + i as u16;
            if y >= area.y + area.height {
                break;
            }
            let rendered = Self::render_diff_line(diff_line, area.width);
            buf.set_line(area.x, y, &rendered, area.width);
        }
    }
}

fn truncate_with_ellipsis(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    if max_chars <= 3 {
        return s.chars().take(max_chars).collect();
    }
    let prefix: String = s.chars().take(max_chars - 3).collect();
    format!("{prefix}...")
}

/// Convert parsed diff lines into styled ratatui Lines (for embedding
/// in the message list rather than rendering as a standalone widget).
pub fn diff_to_lines(diff_text: &str, width: u16) -> Vec<Line<'static>> {
    let parsed = parse_unified_diff(diff_text);
    parsed
        .iter()
        .map(|dl| DiffViewWidget::render_diff_line(dl, width))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_DIFF: &str = "\
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,5 +1,6 @@
 use std::io;

-fn old_function() {
+fn new_function() {
+    println!(\"added line\");
     let x = 1;
 }";

    #[test]
    fn test_parse_unified_diff_line_count() {
        let lines = parse_unified_diff(SAMPLE_DIFF);
        // 2 file headers + 1 hunk header + 7 content lines
        // (use std::io; / empty / -old / +new / +added / let x / })
        assert_eq!(lines.len(), 10);
    }

    #[test]
    fn test_parse_diff_file_headers() {
        let lines = parse_unified_diff(SAMPLE_DIFF);
        assert_eq!(lines[0].kind, DiffLineKind::FileHeader);
        assert_eq!(lines[1].kind, DiffLineKind::FileHeader);
        assert!(lines[0].content.contains("a/src/main.rs"));
        assert!(lines[1].content.contains("b/src/main.rs"));
    }

    #[test]
    fn test_parse_diff_hunk_header() {
        let lines = parse_unified_diff(SAMPLE_DIFF);
        assert_eq!(lines[2].kind, DiffLineKind::HunkHeader);
    }

    #[test]
    fn test_parse_diff_added_lines() {
        let lines = parse_unified_diff(SAMPLE_DIFF);
        let added: Vec<_> = lines
            .iter()
            .filter(|l| l.kind == DiffLineKind::Added)
            .collect();
        assert_eq!(added.len(), 2);
        assert!(added[0].content.contains("new_function"));
        assert!(added[1].content.contains("added line"));
    }

    #[test]
    fn test_parse_diff_removed_lines() {
        let lines = parse_unified_diff(SAMPLE_DIFF);
        let removed: Vec<_> = lines
            .iter()
            .filter(|l| l.kind == DiffLineKind::Removed)
            .collect();
        assert_eq!(removed.len(), 1);
        assert!(removed[0].content.contains("old_function"));
    }

    #[test]
    fn test_parse_diff_context_lines() {
        let lines = parse_unified_diff(SAMPLE_DIFF);
        let context: Vec<_> = lines
            .iter()
            .filter(|l| l.kind == DiffLineKind::Context)
            .collect();
        // "use std::io;", "", "    let x = 1;", "}"
        assert_eq!(context.len(), 4);
    }

    #[test]
    fn test_parse_diff_line_numbers() {
        let lines = parse_unified_diff(SAMPLE_DIFF);
        // First context line after hunk: old=1, new=1
        let first_ctx = lines
            .iter()
            .find(|l| l.kind == DiffLineKind::Context)
            .unwrap();
        assert_eq!(first_ctx.old_lineno, Some(1));
        assert_eq!(first_ctx.new_lineno, Some(1));

        // Added line should have new_lineno but no old_lineno
        let first_add = lines
            .iter()
            .find(|l| l.kind == DiffLineKind::Added)
            .unwrap();
        assert!(first_add.old_lineno.is_none());
        assert!(first_add.new_lineno.is_some());

        // Removed line should have old_lineno but no new_lineno
        let first_rem = lines
            .iter()
            .find(|l| l.kind == DiffLineKind::Removed)
            .unwrap();
        assert!(first_rem.old_lineno.is_some());
        assert!(first_rem.new_lineno.is_none());
    }

    #[test]
    fn test_diff_to_lines_produces_output() {
        let lines = diff_to_lines(SAMPLE_DIFF, 80);
        assert!(!lines.is_empty());
        assert_eq!(lines.len(), 10);
    }

    #[test]
    fn test_parse_hunk_header_numbers() {
        let (old, new) = parse_hunk_header("@@ -1,5 +1,6 @@").unwrap();
        assert_eq!(old, 1);
        assert_eq!(new, 1);

        let (old, new) = parse_hunk_header("@@ -42,10 +55,15 @@ fn test()").unwrap();
        assert_eq!(old, 42);
        assert_eq!(new, 55);
    }

    #[test]
    fn test_parse_empty_diff() {
        let lines = parse_unified_diff("");
        // "".lines() returns an empty iterator in Rust
        assert_eq!(lines.len(), 0);
    }

    #[test]
    fn test_diff_added_color_is_green() {
        let lines = parse_unified_diff(SAMPLE_DIFF);
        let added = lines
            .iter()
            .find(|l| l.kind == DiffLineKind::Added)
            .unwrap();
        let rendered = DiffViewWidget::render_diff_line(added, 80);
        // The marker span should be green
        let has_green = rendered
            .spans
            .iter()
            .any(|s| s.style.fg == Some(Color::Green));
        assert!(has_green, "added lines should be green");
    }

    #[test]
    fn test_diff_removed_color_is_red() {
        let lines = parse_unified_diff(SAMPLE_DIFF);
        let removed = lines
            .iter()
            .find(|l| l.kind == DiffLineKind::Removed)
            .unwrap();
        let rendered = DiffViewWidget::render_diff_line(removed, 80);
        let has_red = rendered
            .spans
            .iter()
            .any(|s| s.style.fg == Some(Color::Red));
        assert!(has_red, "removed lines should be red");
    }
}
