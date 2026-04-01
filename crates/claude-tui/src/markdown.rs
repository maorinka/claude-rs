use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::syntax;

/// Convert a markdown string to styled ratatui Lines.
/// Supports: **bold**, *italic*, `inline code`, ```code blocks``` with syntax
/// highlighting, # headers, - lists
pub fn render_markdown(text: &str) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut in_code_block = false;
    let mut code_language: Option<String> = None;
    let mut code_buffer: Vec<String> = Vec::new();

    for line in text.lines() {
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

        // Headers
        if line.starts_with("### ") {
            lines.push(Line::from(Span::styled(
                line[4..].to_string(),
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            )));
        } else if line.starts_with("## ") {
            lines.push(Line::from(Span::styled(
                line[3..].to_string(),
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            )));
        } else if line.starts_with("# ") {
            lines.push(Line::from(Span::styled(
                line[2..].to_string(),
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            )));
        }
        // List items
        else if line.starts_with("- ") || line.starts_with("* ") {
            lines.push(Line::from(vec![
                Span::styled("  · ", Style::default().fg(Color::DarkGray)),
                Span::raw(render_inline_markdown(&line[2..])),
            ]));
        }
        // Regular text with inline formatting
        else {
            lines.push(render_inline_line(line));
        }
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
        // Inline code
        if chars[i] == '`' {
            if !current.is_empty() {
                spans.push(Span::raw(current.clone()));
                current.clear();
            }
            i += 1;
            let start = i;
            while i < chars.len() && chars[i] != '`' { i += 1; }
            let code: String = chars[start..i].iter().collect();
            spans.push(Span::styled(code, Style::default().fg(Color::Green)));
            if i < chars.len() { i += 1; }
        }
        // Bold
        else if i + 1 < chars.len() && chars[i] == '*' && chars[i + 1] == '*' {
            if !current.is_empty() {
                spans.push(Span::raw(current.clone()));
                current.clear();
            }
            i += 2;
            let start = i;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '*') { i += 1; }
            let bold: String = chars[start..i].iter().collect();
            spans.push(Span::styled(bold, Style::default().add_modifier(Modifier::BOLD)));
            if i + 1 < chars.len() { i += 2; } else { i = chars.len(); }
        }
        // Italic
        else if chars[i] == '*' {
            if !current.is_empty() {
                spans.push(Span::raw(current.clone()));
                current.clear();
            }
            i += 1;
            let start = i;
            while i < chars.len() && chars[i] != '*' { i += 1; }
            let italic: String = chars[start..i].iter().collect();
            spans.push(Span::styled(italic, Style::default().add_modifier(Modifier::ITALIC)));
            if i < chars.len() { i += 1; }
        }
        else {
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
    // Simple pass-through for list items (inline formatting handled elsewhere)
    text.to_string()
}
