use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

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
    /// Cached rendered heights per message (invalidated on push/clear).
    /// Each entry is the number of rendered lines for that message.
    height_cache: Vec<Option<usize>>,
}

impl MessageList {
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
            scroll_offset: 0,
            sticky_bottom: true,
            height_cache: Vec::new(),
        }
    }

    pub fn push(&mut self, msg: MessageEntry) {
        self.messages.push(msg);
        self.height_cache.push(None); // Invalidate for new message
        if self.sticky_bottom {
            // Auto-scroll will be handled in render
        }
    }

    pub fn messages(&self) -> &[MessageEntry] { &self.messages }
    pub fn messages_mut(&mut self) -> &mut Vec<MessageEntry> {
        // Invalidate all cached heights when messages are mutated
        self.height_cache.iter_mut().for_each(|h| *h = None);
        &mut self.messages
    }
    pub fn len(&self) -> usize { self.messages.len() }
    pub fn is_empty(&self) -> bool { self.messages.is_empty() }

    /// Scroll up by N lines (smooth line-level scrolling).
    pub fn scroll_up(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
        self.sticky_bottom = false;
    }

    /// Scroll down by N lines (smooth line-level scrolling).
    pub fn scroll_down(&mut self, lines: usize) {
        self.scroll_offset += lines;
        // sticky_bottom re-enabled in render if at bottom
    }

    /// Scroll up by a full page (viewport height).
    pub fn page_up(&mut self, viewport_height: usize) {
        self.scroll_up(viewport_height.saturating_sub(2)); // Keep 2 lines overlap
    }

    /// Scroll down by a full page (viewport height).
    pub fn page_down(&mut self, viewport_height: usize) {
        self.scroll_down(viewport_height.saturating_sub(2));
    }

    /// Jump to the very top of the message list.
    pub fn scroll_to_top(&mut self) {
        self.scroll_offset = 0;
        self.sticky_bottom = false;
    }

    /// Jump to the very bottom (re-enable sticky bottom).
    pub fn scroll_to_bottom(&mut self) {
        self.sticky_bottom = true;
    }

    pub fn clear(&mut self) {
        self.messages.clear();
        self.height_cache.clear();
        self.scroll_offset = 0;
        self.sticky_bottom = true;
    }

    /// Whether the view is currently pinned to the bottom.
    pub fn is_at_bottom(&self) -> bool {
        self.sticky_bottom
    }
}

pub struct MessageListWidget<'a> {
    list: &'a MessageList,
}

impl<'a> MessageListWidget<'a> {
    pub fn new(list: &'a MessageList) -> Self { Self { list } }
}

/// Detect if text looks like a unified diff (starts with common diff markers).
fn looks_like_diff(text: &str) -> bool {
    let trimmed = text.trim_start();
    trimmed.starts_with("--- ") || trimmed.starts_with("diff --git")
}

/// Render a single message into lines. This is the function used both for
/// display and for height estimation, ensuring consistency.
fn render_message(msg: &MessageEntry, width: u16) -> Vec<Line<'static>> {
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
            // Use markdown rendering with syntax highlighting
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

            // If the output looks like a diff, render it with diff styling
            if looks_like_diff(output) {
                let diff_lines = diff_view::diff_to_lines(output, width.saturating_sub(4));
                for dl in diff_lines {
                    let mut spans = vec![Span::raw("    ")];
                    spans.extend(dl.spans);
                    lines.push(Line::from(spans));
                }
            } else {
                // Show a few lines of output in muted style
                let max_preview_lines = 6;
                for line in output.lines().take(max_preview_lines) {
                    lines.push(Line::from(Span::styled(
                        format!("    {}", line),
                        Style::default().fg(Color::DarkGray),
                    )));
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
            let preview = if text.len() > 100 {
                format!("{}...", &text[..97])
            } else {
                text.clone()
            };
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    "thinking",
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::ITALIC),
                ),
                Span::styled(
                    format!(" {}", preview),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
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

        // Virtual scrolling: render all messages into lines.
        // For truly large histories we could do lazy rendering, but
        // the line rendering is fast enough for typical sessions.
        // The key improvement is smooth line-level scrolling + page/home/end.
        let mut all_lines: Vec<Line> = Vec::new();
        for msg in &self.list.messages {
            let msg_lines = render_message(msg, area.width);
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
