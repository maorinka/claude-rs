use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::syntax;

/// Convert a markdown string to styled ratatui Lines.
/// Supports: **bold**, *italic*, `inline code`, links, ```code blocks``` with
/// syntax highlighting, headers, lists, blockquotes, horizontal rules, and
/// simple GFM tables.
pub fn render_markdown(text: &str) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut in_code_block = false;
    let mut code_language: Option<String> = None;
    let mut code_buffer: Vec<String> = Vec::new();
    let mut table_buffer: Vec<Vec<String>> = Vec::new();

    for line in text.lines() {
        if !is_table_row(line) && !table_buffer.is_empty() {
            flush_table(&mut lines, &mut table_buffer);
        }

        if line.starts_with("```") {
            if in_code_block {
                // End of code block -- flush with syntax highlighting
                let code = code_buffer.join("\n");
                let lang = code_language.as_deref().unwrap_or("");

                // Top separator
                lines.push(Line::from(Span::styled(
                    "─".repeat(40),
                    Style::default().fg(Color::DarkGray),
                )));

                if !code.is_empty() {
                    let highlighted = syntax::highlight_code(&code, lang);
                    // Indent highlighted lines
                    for hl_line in highlighted {
                        let mut spans = vec![Span::raw("  ")];
                        spans.extend(hl_line.spans);
                        lines.push(Line::from(spans));
                    }
                }

                // Bottom separator
                lines.push(Line::from(Span::styled(
                    "─".repeat(40),
                    Style::default().fg(Color::DarkGray),
                )));

                in_code_block = false;
                code_language = None;
                code_buffer.clear();
            } else {
                // Start of code block -- detect language
                in_code_block = true;
                code_language = syntax::detect_language(line);
                code_buffer.clear();
            }
            continue;
        }

        if in_code_block {
            code_buffer.push(line.to_string());
            continue;
        }

        if is_table_row(line) {
            if !is_table_separator(line) {
                table_buffer.push(parse_table_cells(line));
            }
            continue;
        }

        // Headers
        if let Some(stripped) = line
            .strip_prefix("###### ")
            .or_else(|| line.strip_prefix("##### "))
            .or_else(|| line.strip_prefix("#### "))
        {
            lines.push(Line::from(Span::styled(
                stripped.to_string(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )));
        } else if let Some(stripped) = line.strip_prefix("### ") {
            lines.push(Line::from(Span::styled(
                stripped.to_string(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )));
        } else if let Some(stripped) = line.strip_prefix("## ") {
            lines.push(Line::from(Span::styled(
                stripped.to_string(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )));
        } else if let Some(stripped) = line.strip_prefix("# ") {
            lines.push(Line::from(Span::styled(
                stripped.to_string(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            )));
        }
        // List items
        else if line.starts_with("- ") || line.starts_with("* ") {
            lines.push(Line::from(vec![
                Span::styled("  · ", Style::default().fg(Color::DarkGray)),
                Span::raw(render_inline_markdown(&line[2..])),
            ]));
        } else if let Some((indent, marker, item)) = parse_ordered_list(line) {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{}{} ", " ".repeat(indent), marker),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::raw(render_inline_markdown(item)),
            ]));
        } else if let Some(stripped) = line.strip_prefix("> ") {
            lines.push(Line::from(vec![
                Span::styled("│ ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    render_inline_markdown(stripped),
                    Style::default()
                        .fg(Color::Gray)
                        .add_modifier(Modifier::ITALIC),
                ),
            ]));
        } else if is_horizontal_rule(line) {
            lines.push(Line::from(Span::styled(
                "─".repeat(60),
                Style::default().fg(Color::DarkGray),
            )));
        }
        // Regular text with inline formatting
        else {
            lines.push(render_inline_line(line));
        }
    }

    if !table_buffer.is_empty() {
        flush_table(&mut lines, &mut table_buffer);
    }

    // Handle unterminated code block
    if in_code_block && !code_buffer.is_empty() {
        let code = code_buffer.join("\n");
        let lang = code_language.as_deref().unwrap_or("");
        lines.push(Line::from(Span::styled(
            "─".repeat(40),
            Style::default().fg(Color::DarkGray),
        )));
        let highlighted = syntax::highlight_code(&code, lang);
        for hl_line in highlighted {
            let mut spans = vec![Span::raw("  ")];
            spans.extend(hl_line.spans);
            lines.push(Line::from(spans));
        }
    }

    lines
}

/// Parse inline markdown: **bold**, *italic*, `code`
fn render_inline_line(text: &str) -> Line<'static> {
    let mut spans = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        // Links: [label](url). Render as "label (url)" in muted style.
        if chars[i] == '[' {
            if let Some((consumed, label, url)) = parse_link(&chars[i..]) {
                if !current.is_empty() {
                    spans.push(Span::raw(current.clone()));
                    current.clear();
                }
                spans.push(Span::styled(label, Style::default().fg(Color::LightBlue)));
                spans.push(Span::styled(
                    format!(" ({url})"),
                    Style::default().fg(Color::DarkGray),
                ));
                i += consumed;
                continue;
            }
        }

        // Inline code
        if chars[i] == '`' {
            if !current.is_empty() {
                spans.push(Span::raw(current.clone()));
                current.clear();
            }
            i += 1;
            let start = i;
            while i < chars.len() && chars[i] != '`' {
                i += 1;
            }
            let code: String = chars[start..i].iter().collect();
            spans.push(Span::styled(code, Style::default().fg(Color::Green)));
            if i < chars.len() {
                i += 1;
            }
        }
        // Bold
        else if i + 1 < chars.len() && chars[i] == '*' && chars[i + 1] == '*' {
            if !current.is_empty() {
                spans.push(Span::raw(current.clone()));
                current.clear();
            }
            i += 2;
            let start = i;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '*') {
                i += 1;
            }
            let bold: String = chars[start..i].iter().collect();
            spans.push(Span::styled(
                bold,
                Style::default().add_modifier(Modifier::BOLD),
            ));
            if i + 1 < chars.len() {
                i += 2;
            } else {
                i = chars.len();
            }
        }
        // Italic
        else if chars[i] == '*' {
            if !current.is_empty() {
                spans.push(Span::raw(current.clone()));
                current.clear();
            }
            i += 1;
            let start = i;
            while i < chars.len() && chars[i] != '*' {
                i += 1;
            }
            let italic: String = chars[start..i].iter().collect();
            spans.push(Span::styled(
                italic,
                Style::default().add_modifier(Modifier::ITALIC),
            ));
            if i < chars.len() {
                i += 1;
            }
        } else {
            current.push(chars[i]);
            i += 1;
        }
    }

    if !current.is_empty() {
        spans.push(Span::raw(current));
    }

    Line::from(spans)
}

fn render_inline_markdown(text: &str) -> String {
    render_inline_line(text)
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<Vec<_>>()
        .join("")
}

fn parse_ordered_list(line: &str) -> Option<(usize, String, &str)> {
    let indent = line.chars().take_while(|c| c.is_whitespace()).count();
    let trimmed = line.trim_start();
    let dot = trimmed.find(". ")?;
    if dot == 0 || !trimmed[..dot].chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some((
        indent + 2,
        format!("{}.", &trimmed[..dot]),
        &trimmed[dot + 2..],
    ))
}

fn is_horizontal_rule(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.len() >= 3
        && trimmed
            .chars()
            .all(|c| c == '-' || c == '*' || c == '_' || c.is_whitespace())
        && trimmed
            .chars()
            .filter(|c| !c.is_whitespace())
            .all(|c| c == trimmed.chars().find(|ch| !ch.is_whitespace()).unwrap_or(c))
}

fn is_table_row(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with('|') && trimmed.ends_with('|') && trimmed.matches('|').count() >= 2
}

fn is_table_separator(line: &str) -> bool {
    let stripped = line
        .trim()
        .trim_matches('|')
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>();
    !stripped.is_empty() && stripped.chars().all(|c| c == '-' || c == ':' || c == '|')
}

fn parse_table_cells(line: &str) -> Vec<String> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(|cell| render_inline_markdown(cell.trim()))
        .collect()
}

fn flush_table(lines: &mut Vec<Line<'static>>, table: &mut Vec<Vec<String>>) {
    if table.is_empty() {
        return;
    }
    let column_count = table.iter().map(Vec::len).max().unwrap_or(0);
    let mut widths = vec![0usize; column_count];
    for row in table.iter() {
        for (idx, cell) in row.iter().enumerate() {
            widths[idx] = widths[idx].max(cell.chars().count());
        }
    }

    for (row_idx, row) in table.iter().enumerate() {
        let mut rendered = String::new();
        for col in 0..column_count {
            if col > 0 {
                rendered.push_str("  ");
            }
            let cell = row.get(col).map(String::as_str).unwrap_or("");
            rendered.push_str(cell);
            let padding = widths[col].saturating_sub(cell.chars().count());
            rendered.push_str(&" ".repeat(padding));
        }
        let style = if row_idx == 0 {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(rendered, style)));
    }

    table.clear();
}

fn parse_link(chars: &[char]) -> Option<(usize, String, String)> {
    let label_end = chars.iter().position(|c| *c == ']')?;
    if chars.get(label_end + 1) != Some(&'(') {
        return None;
    }
    let url_start = label_end + 2;
    let url_end = chars[url_start..].iter().position(|c| *c == ')')? + url_start;
    let label: String = chars[1..label_end].iter().collect();
    let url: String = chars[url_start..url_end].iter().collect();
    if label.is_empty() || url.is_empty() {
        return None;
    }
    Some((url_end + 1, label, url))
}
