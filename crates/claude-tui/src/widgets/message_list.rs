use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

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
}

impl MessageList {
    pub fn new() -> Self {
        Self { messages: Vec::new(), scroll_offset: 0, sticky_bottom: true }
    }

    pub fn push(&mut self, msg: MessageEntry) {
        self.messages.push(msg);
        if self.sticky_bottom {
            // Auto-scroll will be handled in render
        }
    }

    pub fn messages(&self) -> &[MessageEntry] { &self.messages }
    pub fn messages_mut(&mut self) -> &mut Vec<MessageEntry> { &mut self.messages }
    pub fn len(&self) -> usize { self.messages.len() }
    pub fn is_empty(&self) -> bool { self.messages.is_empty() }

    pub fn scroll_up(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
        self.sticky_bottom = false;
    }

    pub fn scroll_down(&mut self, lines: usize) {
        self.scroll_offset += lines;
        // sticky_bottom re-enabled in render if at bottom
    }

    pub fn scroll_to_bottom(&mut self) {
        self.sticky_bottom = true;
    }

    pub fn clear(&mut self) {
        self.messages.clear();
        self.scroll_offset = 0;
        self.sticky_bottom = true;
    }
}

pub struct MessageListWidget<'a> {
    list: &'a MessageList,
}

impl<'a> MessageListWidget<'a> {
    pub fn new(list: &'a MessageList) -> Self { Self { list } }
}

impl<'a> Widget for MessageListWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 { return; }

        // Render messages as lines
        let mut all_lines: Vec<Line> = Vec::new();

        for msg in &self.list.messages {
            match msg {
                MessageEntry::User { text } => {
                    // Blank line before user message for separation
                    all_lines.push(Line::from(""));
                    all_lines.push(Line::from(vec![
                        Span::styled(
                            " You ",
                            Style::default()
                                .fg(Color::White)
                                .bg(Color::Blue)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ]));
                    // User text indented
                    for line in text.lines() {
                        all_lines.push(Line::from(vec![
                            Span::raw(" "),
                            Span::raw(line.to_string()),
                        ]));
                    }
                }
                MessageEntry::Assistant { text } => {
                    // Blank line before assistant message
                    all_lines.push(Line::from(""));
                    all_lines.push(Line::from(vec![
                        Span::styled(
                            " Claude ",
                            Style::default()
                                .fg(Color::White)
                                .bg(Color::Rgb(180, 100, 60))
                                .add_modifier(Modifier::BOLD),
                        ),
                    ]));
                    // Assistant text indented
                    for line in text.lines() {
                        all_lines.push(Line::from(vec![
                            Span::raw(" "),
                            Span::raw(line.to_string()),
                        ]));
                    }
                }
                MessageEntry::ToolUse { name, input_summary } => {
                    all_lines.push(Line::from(vec![
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
                    all_lines.push(Line::from(vec![
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
                    // Show a few lines of output in muted style
                    let max_preview_lines = 6;
                    for line in output.lines().take(max_preview_lines) {
                        all_lines.push(Line::from(Span::styled(
                            format!("    {}", line),
                            Style::default().fg(Color::DarkGray),
                        )));
                    }
                    let output_line_count = output.lines().count();
                    if output_line_count > max_preview_lines {
                        all_lines.push(Line::from(Span::styled(
                            format!("    ... ({} more lines)", output_line_count - max_preview_lines),
                            Style::default().fg(Color::DarkGray),
                        )));
                    }
                }
                MessageEntry::Thinking { text } => {
                    let preview = if text.len() > 100 {
                        format!("{}...", &text[..97])
                    } else {
                        text.clone()
                    };
                    all_lines.push(Line::from(vec![
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
                    all_lines.push(Line::from(vec![
                        Span::raw(" "),
                        Span::styled(
                            text.clone(),
                            Style::default().fg(Color::Yellow),
                        ),
                    ]));
                }
            }
        }

        // Virtual scrolling: determine visible range
        let total_lines = all_lines.len();
        let visible_height = area.height as usize;

        let scroll = if self.list.sticky_bottom {
            total_lines.saturating_sub(visible_height)
        } else {
            self.list.scroll_offset.min(total_lines.saturating_sub(visible_height))
        };

        let visible = &all_lines[scroll..total_lines.min(scroll + visible_height)];
        for (i, line) in visible.iter().enumerate() {
            let y = area.y + i as u16;
            if y >= area.y + area.height { break; }
            buf.set_line(area.x, y, line, area.width);
        }
    }
}
