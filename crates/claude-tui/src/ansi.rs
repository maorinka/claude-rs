use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

pub fn parse_ansi(text: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut current_style = Style::default();
    let mut buf = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\x1b' && i + 1 < chars.len() && chars[i + 1] == '[' {
            if !buf.is_empty() {
                spans.push(Span::styled(buf.clone(), current_style));
                buf.clear();
            }
            i += 2;
            let mut code = String::new();
            while i < chars.len() && chars[i] != 'm' {
                code.push(chars[i]);
                i += 1;
            }
            if i < chars.len() {
                i += 1;
            }
            for part in code.split(';') {
                match part {
                    "0" => current_style = Style::default(),
                    "1" => current_style = current_style.add_modifier(Modifier::BOLD),
                    "3" => current_style = current_style.add_modifier(Modifier::ITALIC),
                    "4" => current_style = current_style.add_modifier(Modifier::UNDERLINED),
                    "31" => current_style = current_style.fg(Color::Red),
                    "32" => current_style = current_style.fg(Color::Green),
                    "33" => current_style = current_style.fg(Color::Yellow),
                    "34" => current_style = current_style.fg(Color::Blue),
                    "35" => current_style = current_style.fg(Color::Magenta),
                    "36" => current_style = current_style.fg(Color::Cyan),
                    "37" => current_style = current_style.fg(Color::White),
                    _ => {}
                }
            }
        } else {
            buf.push(chars[i]);
            i += 1;
        }
    }
    if !buf.is_empty() {
        spans.push(Span::styled(buf, current_style));
    }
    spans
}
