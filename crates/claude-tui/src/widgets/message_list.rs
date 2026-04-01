use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::ansi::parse_ansi;
use crate::links;
use crate::markdown::render_markdown;
use crate::theme::Theme;
use crate::widgets::diff_view;

/// The circle indicator shown next to assistant text and tool use lines.
/// On macOS the original uses ⏺ (U+23FA), on other platforms ● (U+25CF).
#[cfg(target_os = "macos")]
const BLACK_CIRCLE: &str = "\u{23FA}";
#[cfg(not(target_os = "macos"))]
const BLACK_CIRCLE: &str = "\u{25CF}";

/// The return symbol used for tool result indentation (MessageResponse).
/// The original uses "  ⎿  " — two spaces, ⎿, two spaces.
const TOOL_RESULT_PREFIX: &str = "  \u{23BF}  ";

/// Teardrop asterisk used for system/compact messages in the original.
#[allow(dead_code)]
const TEARDROP_ASTERISK: &str = "\u{273B}";

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
    theme: Option<&'a Theme>,
}

impl<'a> MessageListWidget<'a> {
    pub fn new(list: &'a MessageList) -> Self {
        Self {
            show_thinking: list.show_thinking,
            list,
            theme: None,
        }
    }

    pub fn theme(mut self, theme: &'a Theme) -> Self {
        self.theme = Some(theme);
        self
    }
}

fn looks_like_diff(text: &str) -> bool {
    let trimmed = text.trim_start();
    trimmed.starts_with("--- ") || trimmed.starts_with("diff --git")
}

fn contains_ansi(text: &str) -> bool {
    text.contains('\x1b')
}

fn render_tool_output_line(line_text: &str, theme: &Theme) -> Line<'static> {
    // Tool result lines are prefixed with the ⎿ indicator (dim)
    // Content is also dimmed like the original
    if contains_ansi(line_text) {
        let mut spans = vec![Span::styled(
            TOOL_RESULT_PREFIX.to_string(),
            Style::default().fg(theme.inactive).add_modifier(Modifier::DIM),
        )];
        spans.extend(parse_ansi(line_text));
        Line::from(spans)
    } else {
        let paths = links::find_file_paths(line_text);
        if paths.is_empty() {
            Line::from(Span::styled(
                format!("{}{}", TOOL_RESULT_PREFIX, line_text),
                Style::default().fg(theme.inactive).add_modifier(Modifier::DIM),
            ))
        } else {
            let mut spans: Vec<Span<'static>> = vec![Span::styled(
                TOOL_RESULT_PREFIX.to_string(),
                Style::default().fg(theme.inactive).add_modifier(Modifier::DIM),
            )];
            let mut last_end = 0;
            for (start, end) in &paths {
                if *start > last_end {
                    spans.push(Span::styled(
                        line_text[last_end..*start].to_string(),
                        Style::default().fg(theme.inactive).add_modifier(Modifier::DIM),
                    ));
                }
                spans.push(Span::styled(
                    line_text[*start..*end].to_string(),
                    Style::default()
                        .fg(Color::Rgb(0, 204, 204)) // Bright cyan for links
                        .add_modifier(Modifier::UNDERLINED),
                ));
                last_end = *end;
            }
            if last_end < line_text.len() {
                spans.push(Span::styled(
                    line_text[last_end..].to_string(),
                    Style::default().fg(theme.inactive).add_modifier(Modifier::DIM),
                ));
            }
            Line::from(spans)
        }
    }
}

fn render_message(msg: &MessageEntry, width: u16, show_thinking: bool, theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    match msg {
        MessageEntry::User { text } => {
            // Original: marginTop=1 (blank line), then user text with
            // backgroundColor=userMessageBackground, paddingRight=1.
            // No badge — just the text on a colored background.
            lines.push(Line::from(""));
            for line in text.lines() {
                // User text renders with a background color on the full line
                lines.push(Line::from(vec![
                    Span::styled(
                        format!(" {}", line),
                        Style::default()
                            .bg(theme.user_message_bg),
                    ),
                ]));
            }
        }
        MessageEntry::Assistant { text } => {
            // Original: marginTop=1 (blank line), then BLACK_CIRCLE in
            // minWidth=2 followed by markdown content.
            // The dot is in the 'text' color (white on dark).
            lines.push(Line::from(""));

            let md_lines = render_markdown(text);
            for (i, md_line) in md_lines.iter().enumerate() {
                let prefix = if i == 0 {
                    // First line gets the circle indicator
                    Span::styled(
                        format!("{} ", BLACK_CIRCLE),
                        Style::default().fg(theme.text),
                    )
                } else {
                    // Continuation lines get 2-space indent (minWidth=2)
                    Span::raw("  ".to_string())
                };
                let mut spans = vec![prefix];
                spans.extend(md_line.spans.clone());
                lines.push(Line::from(spans));
            }
        }
        MessageEntry::ToolUse { name, input_summary } => {
            // Original: BLACK_CIRCLE (or ToolUseLoader dot) + bold tool name + (summary)
            // marginTop=1 if addMargin. Tool name is bold, summary in parens.
            lines.push(Line::from(vec![
                // Circle indicator
                Span::styled(
                    format!("{} ", BLACK_CIRCLE),
                    Style::default().fg(theme.text),
                ),
                // Bold tool name
                Span::styled(
                    name.clone(),
                    Style::default()
                        .fg(theme.text)
                        .add_modifier(Modifier::BOLD),
                ),
                // Input summary in parentheses
                if input_summary.is_empty() {
                    Span::raw("")
                } else {
                    Span::styled(
                        format!(" ({})", input_summary),
                        Style::default().fg(theme.inactive),
                    )
                },
            ]));
        }
        MessageEntry::ToolResult { name: _, output, is_error } => {
            // Original: MessageResponse renders "  ⎿  " prefix (dim) then content.
            // Success results show tool output dimmed.
            // Error results show the error text in error color.
            if *is_error {
                // Error: show error indicator with ⎿ prefix
                lines.push(Line::from(vec![
                    Span::styled(
                        TOOL_RESULT_PREFIX.to_string(),
                        Style::default().fg(theme.inactive).add_modifier(Modifier::DIM),
                    ),
                    Span::styled(
                        output.lines().next().unwrap_or("Error").to_string(),
                        Style::default().fg(theme.error),
                    ),
                ]));
                // Show remaining error lines
                for line in output.lines().skip(1).take(5) {
                    lines.push(Line::from(Span::styled(
                        format!("{}{}", TOOL_RESULT_PREFIX, line),
                        Style::default().fg(theme.error),
                    )));
                }
            } else if looks_like_diff(output) {
                let diff_lines = diff_view::diff_to_lines(output, width.saturating_sub(6));
                for dl in diff_lines {
                    let mut spans = vec![Span::styled(
                        TOOL_RESULT_PREFIX.to_string(),
                        Style::default().fg(theme.inactive).add_modifier(Modifier::DIM),
                    )];
                    spans.extend(dl.spans);
                    lines.push(Line::from(spans));
                }
            } else {
                let max_preview_lines = 6;
                for line in output.lines().take(max_preview_lines) {
                    lines.push(render_tool_output_line(line, theme));
                }
                let output_line_count = output.lines().count();
                if output_line_count > max_preview_lines {
                    lines.push(Line::from(Span::styled(
                        format!("{}... ({} more lines)", TOOL_RESULT_PREFIX, output_line_count - max_preview_lines),
                        Style::default().fg(theme.inactive).add_modifier(Modifier::DIM),
                    )));
                }
            }
        }
        MessageEntry::Thinking { text } => {
            // Original: "∴ Thinking" in dimColor + italic.
            // When expanded (verbose/show_thinking): thinking text in dim markdown, paddingLeft=2.
            // When collapsed: "∴ Thinking (ctrl+o to expand)" in dim+italic.
            if show_thinking {
                lines.push(Line::from(vec![
                    Span::styled(
                        "\u{2234} Thinking\u{2026}".to_string(),
                        Style::default()
                            .fg(theme.thinking)
                            .add_modifier(Modifier::DIM | Modifier::ITALIC),
                    ),
                ]));
                // Content indented by 2 (paddingLeft=2 in original)
                for line in text.lines() {
                    lines.push(Line::from(vec![
                        Span::raw("  "),
                        Span::styled(
                            line.to_string(),
                            Style::default()
                                .fg(theme.thinking)
                                .add_modifier(Modifier::DIM),
                        ),
                    ]));
                }
            } else {
                lines.push(Line::from(vec![
                    Span::styled(
                        "\u{2234} Thinking".to_string(),
                        Style::default()
                            .fg(theme.thinking)
                            .add_modifier(Modifier::DIM | Modifier::ITALIC),
                    ),
                    Span::styled(
                        " (ctrl+o to expand)".to_string(),
                        Style::default()
                            .fg(theme.thinking)
                            .add_modifier(Modifier::DIM),
                    ),
                ]));
            }
        }
        MessageEntry::System { text } => {
            // System messages: plain text in inactive/muted color
            lines.push(Line::from(vec![
                Span::styled(
                    text.clone(),
                    Style::default().fg(theme.inactive),
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
        let fallback = crate::theme::dark_theme();
        let theme = self.theme.unwrap_or(&fallback);

        let mut all_lines: Vec<Line> = Vec::new();
        for msg in &self.list.messages {
            let msg_lines = render_message(msg, area.width, show_thinking, theme);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn black_circle_char() {
        // Verify the correct Unicode character is used
        #[cfg(target_os = "macos")]
        assert_eq!(BLACK_CIRCLE, "\u{23FA}");
        #[cfg(not(target_os = "macos"))]
        assert_eq!(BLACK_CIRCLE, "\u{25CF}");
    }

    #[test]
    fn tool_result_prefix_format() {
        // "  ⎿  " — matches the original MessageResponse
        assert!(TOOL_RESULT_PREFIX.contains('\u{23BF}'));
        assert_eq!(TOOL_RESULT_PREFIX.len(), "  \u{23BF}  ".len());
    }

    #[test]
    fn user_message_has_blank_line_before() {
        let theme = crate::theme::dark_theme();
        let lines = render_message(
            &MessageEntry::User { text: "hello".to_string() },
            80,
            false,
            &theme,
        );
        // First line should be blank (margin)
        assert!(lines[0].spans.is_empty() || lines[0].to_string().trim().is_empty());
    }

    #[test]
    fn assistant_message_has_circle() {
        let theme = crate::theme::dark_theme();
        let lines = render_message(
            &MessageEntry::Assistant { text: "hi".to_string() },
            80,
            false,
            &theme,
        );
        // Should have blank line + content line with circle
        assert!(lines.len() >= 2);
        let content_line = &lines[1];
        let text = content_line.to_string();
        assert!(text.contains(BLACK_CIRCLE));
    }

    #[test]
    fn tool_use_has_bold_name() {
        let theme = crate::theme::dark_theme();
        let lines = render_message(
            &MessageEntry::ToolUse {
                name: "Read".to_string(),
                input_summary: "/foo.rs".to_string(),
            },
            80,
            false,
            &theme,
        );
        assert!(!lines.is_empty());
        let text = lines[0].to_string();
        assert!(text.contains("Read"));
        assert!(text.contains("(/foo.rs)"));
    }

    #[test]
    fn thinking_collapsed_shows_ctrl_o() {
        let theme = crate::theme::dark_theme();
        let lines = render_message(
            &MessageEntry::Thinking { text: "let me think...".to_string() },
            80,
            false, // collapsed
            &theme,
        );
        let text: String = lines.iter().map(|l| l.to_string()).collect();
        assert!(text.contains("Thinking"));
        assert!(text.contains("ctrl+o to expand"));
    }

    #[test]
    fn thinking_expanded_shows_content() {
        let theme = crate::theme::dark_theme();
        let lines = render_message(
            &MessageEntry::Thinking { text: "let me think about this".to_string() },
            80,
            true, // expanded
            &theme,
        );
        let text: String = lines.iter().map(|l| l.to_string()).collect();
        assert!(text.contains("Thinking"));
        assert!(text.contains("let me think about this"));
    }
}
