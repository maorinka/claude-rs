use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::ansi::parse_ansi;
use crate::links;
use crate::markdown::render_markdown;
use crate::widgets::diff_view;

#[derive(Clone, Debug)]
pub enum MessageEntry {
    User { text: String },
    Assistant { text: String },
    ToolUse { name: String, input_summary: String },
    ToolResult { name: String, output: String, is_error: bool },
    Thinking { text: String },
    System { text: String },
}

pub struct MessageList {
    messages: Vec<MessageEntry>,
    scroll_offset: usize,
    sticky_bottom: bool,
    /// Whether thinking blocks are visible (toggled with Ctrl+O).
    show_thinking: bool,
    /// Cached rendered heights per message (invalidated on push/clear).
    height_cache: Vec<Option<usize>>,
}

impl MessageList {
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
            scroll_offset: 0,
            sticky_bottom: true,
            show_thinking: false,
            height_cache: Vec::new(),
        }
    }

    pub fn push(&mut self, msg: MessageEntry) {
        self.messages.push(msg);
        self.height_cache.push(None);
        if self.sticky_bottom {
            // Auto-scroll will be handled in render
        }
    }

    pub fn messages(&self) -> &[MessageEntry] { &self.messages }
    pub fn messages_mut(&mut self) -> &mut Vec<MessageEntry> {
        self.height_cache.iter_mut().for_each(|h| *h = None);
        &mut self.messages
    }
    pub fn len(&self) -> usize { self.messages.len() }
    pub fn is_empty(&self) -> bool { self.messages.is_empty() }

    pub fn scroll_up(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
        self.sticky_bottom = false;
    }

    pub fn scroll_down(&mut self, lines: usize) {
        self.scroll_offset += lines;
    }

    pub fn page_up(&mut self, viewport_height: usize) {
        self.scroll_up(viewport_height.saturating_sub(2));
    }

    pub fn page_down(&mut self, viewport_height: usize) {
        self.scroll_down(viewport_height.saturating_sub(2));
    }

    pub fn scroll_to_top(&mut self) {
        self.scroll_offset = 0;
        self.sticky_bottom = false;
    }

    pub fn scroll_to_bottom(&mut self) {
        self.sticky_bottom = true;
    }

    pub fn clear(&mut self) {
        self.messages.clear();
        self.height_cache.clear();
        self.scroll_offset = 0;
        self.sticky_bottom = true;
    }

    pub fn is_at_bottom(&self) -> bool {
        self.sticky_bottom
    }

    /// Toggle visibility of thinking blocks (Ctrl+O).
    pub fn toggle_thinking(&mut self) {
        self.show_thinking = !self.show_thinking;
        self.height_cache.iter_mut().for_each(|h| *h = None);
    }

    /// Whether thinking blocks are currently visible.
    pub fn show_thinking(&self) -> bool {
        self.show_thinking
    }
}

pub struct MessageListWidget<'a> {
    list: &'a MessageList,
    show_thinking: bool,
}

impl<'a> MessageListWidget<'a> {
    pub fn new(list: &'a MessageList) -> Self {
        Self {
            show_thinking: list.show_thinking,
            list,
        }
    }
}

fn looks_like_diff(text: &str) -> bool {
    let trimmed = text.trim_start();
    trimmed.starts_with("--- ") || trimmed.starts_with("diff --git")
}

fn contains_ansi(text: &str) -> bool {
    text.contains('\x1b')
}

fn render_tool_output_line(line_text: &str) -> Line<'static> {
    if contains_ansi(line_text) {
        let mut spans = vec![Span::raw("    ".to_string())];
        spans.extend(parse_ansi(line_text));
        Line::from(spans)
    } else {
        let paths = links::find_file_paths(line_text);
        if paths.is_empty() {
            Line::from(Span::styled(
                format!("    {}", line_text),
                Style::default().fg(Color::DarkGray),
            ))
        } else {
            let mut spans: Vec<Span<'static>> = vec![Span::raw("    ".to_string())];
            let mut last_end = 0;
            for (start, end) in &paths {
                if *start > last_end {
                    spans.push(Span::styled(
                        line_text[last_end..*start].to_string(),
                        Style::default().fg(Color::DarkGray),
                    ));
                }
                spans.push(Span::styled(
                    line_text[*start..*end].to_string(),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::UNDERLINED),
                ));
                last_end = *end;
            }
            if last_end < line_text.len() {
                spans.push(Span::styled(
                    line_text[last_end..].to_string(),
                    Style::default().fg(Color::DarkGray),
                ));
            }
            Line::from(spans)
        }
    }
}

fn render_message(msg: &MessageEntry, width: u16, show_thinking: bool) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    match msg {
        MessageEntry::User { text } => {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled(
                    " You ",
                    Style::default()
                        .fg(Color::White)
                        .bg(Color::Blue)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
            for line in text.lines() {
                lines.push(Line::from(vec![
                    Span::raw(" "),
                    Span::raw(line.to_string()),
                ]));
            }
        }
        MessageEntry::Assistant { text } => {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled(
                    " Claude ",
                    Style::default()
                        .fg(Color::White)
                        .bg(Color::Rgb(180, 100, 60))
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
            let md_lines = render_markdown(text);
            for md_line in md_lines {
                let mut spans = vec![Span::raw(" ")];
                spans.extend(md_line.spans);
                lines.push(Line::from(spans));
            }
        }
        MessageEntry::ToolUse { name, input_summary } => {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    format!(" {} ", name),
                    Style::default()
                        .fg(Color::White)
                        .bg(Color::Magenta),
                ),
                Span::raw(" "),
                Span::styled(input_summary.clone(), Style::default().fg(Color::DarkGray)),
            ]));
        }
        MessageEntry::ToolResult { name: _, output, is_error } => {
            let indicator = if *is_error { "\u{2718}" } else { "\u{2714}" };
            let color = if *is_error { Color::Red } else { Color::Green };
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    format!("{} ", indicator),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    if *is_error { "Error" } else { "Done" },
                    Style::default().fg(color),
                ),
            ]));

            if looks_like_diff(output) {
                let diff_lines = diff_view::diff_to_lines(output, width.saturating_sub(4));
                for dl in diff_lines {
                    let mut spans = vec![Span::raw("    ")];
                    spans.extend(dl.spans);
                    lines.push(Line::from(spans));
                }
            } else {
                let max_preview_lines = 6;
                for line in output.lines().take(max_preview_lines) {
                    lines.push(render_tool_output_line(line));
                }
                let output_line_count = output.lines().count();
                if output_line_count > max_preview_lines {
                    lines.push(Line::from(Span::styled(
                        format!("    ... ({} more lines)", output_line_count - max_preview_lines),
                        Style::default().fg(Color::DarkGray),
                    )));
                }
            }
        }
        MessageEntry::Thinking { text } => {
            if show_thinking {
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(
                        "\u{2234} Thinking\u{2026}".to_string(),
                        Style::default()
                            .fg(Color::DarkGray)
                            .add_modifier(Modifier::ITALIC),
                    ),
                ]));
                for line in text.lines() {
                    lines.push(Line::from(vec![
                        Span::raw("    "),
                        Span::styled(
                            line.to_string(),
                            Style::default().fg(Color::DarkGray),
                        ),
                    ]));
                }
            } else {
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(
                        "\u{2234} Thinking".to_string(),
                        Style::default()
                            .fg(Color::DarkGray)
                            .add_modifier(Modifier::ITALIC),
                    ),
                    Span::styled(
                        " (ctrl+o to expand)".to_string(),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]));
            }
        }
        MessageEntry::System { text } => {
            lines.push(Line::from(vec![
                Span::raw(" "),
                Span::styled(
                    text.clone(),
                    Style::default().fg(Color::Yellow),
                ),
            ]));
        }
    }

    lines
}

impl<'a> Widget for MessageListWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 { return; }

        let visible_height = area.height as usize;
        let show_thinking = self.show_thinking;

        let mut all_lines: Vec<Line> = Vec::new();
        for msg in &self.list.messages {
            let msg_lines = render_message(msg, area.width, show_thinking);
            all_lines.extend(msg_lines);
        }

        let total_lines = all_lines.len();

        let scroll = if self.list.sticky_bottom {
            total_lines.saturating_sub(visible_height)
        } else {
            self.list.scroll_offset.min(total_lines.saturating_sub(visible_height))
        };

        let end = total_lines.min(scroll + visible_height);
        let visible = &all_lines[scroll..end];
        for (i, line) in visible.iter().enumerate() {
            let y = area.y + i as u16;
            if y >= area.y + area.height { break; }
            buf.set_line(area.x, y, line, area.width);
        }
    }
}
