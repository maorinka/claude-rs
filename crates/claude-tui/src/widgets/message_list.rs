use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;
use unicode_width::UnicodeWidthStr;

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
    /// Welcome header shown at the start of a conversation.
    Logo {
        model: String,
        cwd: String,
    },
    User {
        text: String,
    },
    Assistant {
        text: String,
    },
    ToolUse {
        name: String,
        input_summary: String,
        /// Tool use ID for pairing with ToolResult
        tool_use_id: String,
    },
    ToolResult {
        name: String,
        output: String,
        is_error: bool,
        /// Tool use ID this result belongs to
        tool_use_id: String,
    },
    Thinking {
        text: String,
    },
    System {
        text: String,
    },
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
        // For ToolResult, insert right after its matching ToolUse (not at end)
        if let MessageEntry::ToolResult { ref tool_use_id, .. } = msg {
            if let Some(pos) = self.messages.iter().rposition(|m| {
                matches!(m, MessageEntry::ToolUse { tool_use_id: ref id, .. } if id == tool_use_id)
            }) {
                // Insert after the ToolUse (and after any existing ToolResult for it)
                let mut insert_at = pos + 1;
                while insert_at < self.messages.len() {
                    if matches!(&self.messages[insert_at], MessageEntry::ToolResult { tool_use_id: ref id, .. } if id == tool_use_id) {
                        insert_at += 1;
                    } else {
                        break;
                    }
                }
                self.messages.insert(insert_at, msg);
                self.height_cache.insert(insert_at, None);
                return;
            }
        }
        self.messages.push(msg);
        self.height_cache.push(None);
    }

    pub fn messages(&self) -> &[MessageEntry] {
        &self.messages
    }
    pub fn messages_mut(&mut self) -> &mut Vec<MessageEntry> {
        self.height_cache.iter_mut().for_each(|h| *h = None);
        &mut self.messages
    }
    pub fn len(&self) -> usize {
        self.messages.len()
    }
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

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

/// Word-wrap a plain text string to fit within `max_width` display columns.
/// Uses `textwrap` for proper word-boundary wrapping with Unicode width awareness.
fn word_wrap(text: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 {
        return vec![text.to_string()];
    }
    let width = UnicodeWidthStr::width(text);
    if width <= max_width {
        return vec![text.to_string()];
    }

    let options = textwrap::Options::new(max_width)
        .break_words(true)
        .word_splitter(textwrap::WordSplitter::NoHyphenation);
    let wrapped = textwrap::wrap(text, &options);
    wrapped.into_iter().map(|cow| cow.into_owned()).collect()
}

/// Wrap a `Line` of styled `Span`s so that no output line exceeds `max_width`
/// display columns. Continuation lines are prefixed with `indent` spaces.
/// Each span's style is preserved across the wrap boundary.
fn wrap_spans(line: &Line<'static>, max_width: usize, indent: usize) -> Vec<Line<'static>> {
    if max_width == 0 {
        return vec![line.clone()];
    }

    // Fast path: check if the line already fits.
    let total_width: usize = line.spans.iter().map(|s| UnicodeWidthStr::width(s.content.as_ref())).sum();
    if total_width <= max_width {
        return vec![line.clone()];
    }

    // Flatten all spans into a list of (char, style) preserving styled regions,
    // then re-wrap at word boundaries.
    let mut chars_styles: Vec<(char, Style)> = Vec::new();
    for span in &line.spans {
        for ch in span.content.chars() {
            chars_styles.push((ch, span.style));
        }
    }

    // Build the plain text to wrap.
    let plain: String = chars_styles.iter().map(|(c, _)| *c).collect();
    let wrapped_lines = word_wrap(&plain, max_width);

    let mut result = Vec::new();
    let mut char_offset = 0;
    for (line_idx, wrapped_text) in wrapped_lines.iter().enumerate() {
        let mut spans: Vec<Span<'static>> = Vec::new();

        // Add indent for continuation lines.
        if line_idx > 0 && indent > 0 {
            spans.push(Span::raw(" ".repeat(indent)));
        }

        let mut current_text = String::new();
        let mut current_style: Option<Style> = None;

        for ch in wrapped_text.chars() {
            if char_offset < chars_styles.len() {
                let (_, style) = chars_styles[char_offset];
                if current_style.map_or(false, |s| s != style) {
                    if !current_text.is_empty() {
                        spans.push(Span::styled(current_text.clone(), current_style.unwrap()));
                        current_text.clear();
                    }
                }
                current_style = Some(style);
                current_text.push(ch);
                char_offset += 1;
            }
        }
        if !current_text.is_empty() {
            spans.push(Span::styled(
                current_text,
                current_style.unwrap_or_default(),
            ));
        }

        result.push(Line::from(spans));
    }

    if result.is_empty() {
        vec![line.clone()]
    } else {
        result
    }
}

/// Wrap a sequence of rendered markdown `Line`s to fit within `max_width`.
/// The `indent` is added to continuation lines (e.g., 2 for assistant text).
fn wrap_md_lines(
    md_lines: Vec<Line<'static>>,
    max_width: usize,
    indent: usize,
) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    for line in &md_lines {
        let wrapped = wrap_spans(line, max_width, indent);
        out.extend(wrapped);
    }
    out
}

fn looks_like_diff(text: &str) -> bool {
    let trimmed = text.trim_start();
    trimmed.starts_with("--- ") || trimmed.starts_with("diff --git")
}

fn contains_ansi(text: &str) -> bool {
    text.contains('\x1b')
}

/// Width of the tool result prefix "  ⎿  " in display columns.
const TOOL_RESULT_PREFIX_WIDTH: usize = 5;

fn render_tool_output_lines(line_text: &str, width: usize, theme: &Theme) -> Vec<Line<'static>> {
    // Tool result lines are prefixed with the ⎿ indicator (dim).
    // Content wraps within (width - prefix_width).
    let content_width = width.saturating_sub(TOOL_RESULT_PREFIX_WIDTH);
    let dim_style = Style::default()
        .fg(theme.inactive)
        .add_modifier(Modifier::DIM);
    let prefix_span = Span::styled(TOOL_RESULT_PREFIX.to_string(), dim_style);
    let cont_prefix = Span::styled("     ".to_string(), dim_style); // 5 spaces for continuation

    // Content uses normal text style (not dim) — only the ⎿ prefix is dim.
    // This matches the TS MessageResponse which renders children normally.
    let content_style = Style::default().fg(theme.inactive);

    if contains_ansi(line_text) {
        let mut spans = vec![prefix_span];
        spans.extend(parse_ansi(line_text));
        // ANSI lines are already formatted; just return as-is.
        vec![Line::from(spans)]
    } else {
        let paths = links::find_file_paths(line_text);
        // Word-wrap the text content first.
        let wrapped = word_wrap(line_text, content_width.max(10));
        let mut result = Vec::new();
        for (i, wrapped_line) in wrapped.iter().enumerate() {
            let pfx = if i == 0 {
                prefix_span.clone()
            } else {
                cont_prefix.clone()
            };

            if paths.is_empty() {
                result.push(Line::from(vec![
                    pfx,
                    Span::styled(wrapped_line.clone(), content_style),
                ]));
            } else {
                // Re-check paths within this wrapped segment.
                let seg_paths = links::find_file_paths(wrapped_line);
                if seg_paths.is_empty() {
                    result.push(Line::from(vec![
                        pfx,
                        Span::styled(wrapped_line.clone(), content_style),
                    ]));
                } else {
                    let mut spans: Vec<Span<'static>> = vec![pfx];
                    let mut last_end = 0;
                    for (start, end) in &seg_paths {
                        if *start > last_end {
                            spans.push(Span::styled(
                                wrapped_line[last_end..*start].to_string(),
                                content_style,
                            ));
                        }
                        spans.push(Span::styled(
                            wrapped_line[*start..*end].to_string(),
                            Style::default()
                                .fg(Color::Rgb(0, 204, 204))
                                .add_modifier(Modifier::UNDERLINED),
                        ));
                        last_end = *end;
                    }
                    if last_end < wrapped_line.len() {
                        spans.push(Span::styled(
                            wrapped_line[last_end..].to_string(),
                            content_style,
                        ));
                    }
                    result.push(Line::from(spans));
                }
            }
        }
        result
    }
}

fn render_message(
    msg: &MessageEntry,
    width: u16,
    show_thinking: bool,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    match msg {
        MessageEntry::Logo { model, cwd } => {
            // Welcome header matching TS LogoV2 condensed format:
            //   ╭─────────────────────────╮
            //   │  ✻  Claude Code         │
            //   │                         │
            //   │  Model: claude-sonnet   │
            //   │  cwd: ~/projects/foo    │
            //   ╰─────────────────────────╯
            let box_width = (width as usize).min(50);
            let inner = box_width.saturating_sub(4); // 2 border + 2 padding

            // Top border
            lines.push(Line::from(Span::styled(
                format!(
                    "  \u{256D}{}\u{256E}",
                    "\u{2500}".repeat(box_width.saturating_sub(2))
                ),
                Style::default().fg(theme.claude),
            )));

            // Title line: "  ✻  Claude Code"
            let title = "\u{273B} Claude Code";
            let pad = inner.saturating_sub(title.len());
            lines.push(Line::from(vec![
                Span::styled("  \u{2502} ", Style::default().fg(theme.claude)),
                Span::styled(
                    title.to_string(),
                    Style::default()
                        .fg(theme.claude)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("{}", " ".repeat(pad)), Style::default()),
                Span::styled(" \u{2502}", Style::default().fg(theme.claude)),
            ]));

            // Blank line
            lines.push(Line::from(vec![
                Span::styled("  \u{2502}", Style::default().fg(theme.claude)),
                Span::raw(" ".repeat(box_width.saturating_sub(2))),
                Span::styled("\u{2502}", Style::default().fg(theme.claude)),
            ]));

            // Model line
            let model_text = format!("Model: {}", model);
            let model_text = if model_text.len() > inner {
                format!("{}...", &model_text[..inner.saturating_sub(3)])
            } else {
                model_text
            };
            let model_pad = inner.saturating_sub(model_text.len());
            lines.push(Line::from(vec![
                Span::styled("  \u{2502} ", Style::default().fg(theme.claude)),
                Span::styled(model_text, Style::default().fg(theme.inactive)),
                Span::raw(" ".repeat(model_pad)),
                Span::styled(" \u{2502}", Style::default().fg(theme.claude)),
            ]));

            // CWD line
            let cwd_text = format!("cwd: {}", cwd);
            let cwd_text = if cwd_text.len() > inner {
                format!("{}...", &cwd_text[..inner.saturating_sub(3)])
            } else {
                cwd_text
            };
            let cwd_pad = inner.saturating_sub(cwd_text.len());
            lines.push(Line::from(vec![
                Span::styled("  \u{2502} ", Style::default().fg(theme.claude)),
                Span::styled(cwd_text, Style::default().fg(theme.inactive)),
                Span::raw(" ".repeat(cwd_pad)),
                Span::styled(" \u{2502}", Style::default().fg(theme.claude)),
            ]));

            // Bottom border
            lines.push(Line::from(Span::styled(
                format!(
                    "  \u{2570}{}\u{256F}",
                    "\u{2500}".repeat(box_width.saturating_sub(2))
                ),
                Style::default().fg(theme.claude),
            )));

            lines.push(Line::from(""));
        }
        MessageEntry::User { text } => {
            // Original: marginTop=1 (blank line), then user text with
            // backgroundColor=userMessageBackground filling the full width.
            lines.push(Line::from(""));
            let content_width = width as usize;
            // Wrap at (width - 2) to leave 1 col padding on left, 1 spare
            let wrap_at = content_width.saturating_sub(2).max(1);
            for line in text.lines() {
                for wrapped in word_wrap(line, wrap_at) {
                    // Pad to full width so background fills the entire row.
                    let display = format!(" {}", wrapped);
                    let display_w = UnicodeWidthStr::width(display.as_str());
                    let pad = content_width.saturating_sub(display_w);
                    lines.push(Line::from(vec![Span::styled(
                        format!("{}{}", display, " ".repeat(pad)),
                        Style::default().bg(theme.user_message_bg),
                    )]));
                }
            }
        }
        MessageEntry::Assistant { text } => {
            // Original TS: marginTop=1 (blank line), then BLACK_CIRCLE in
            // minWidth=2 followed by markdown content. The circle prefix is 2
            // columns wide ("⏺ "), so content wraps at (width - 2).
            lines.push(Line::from(""));

            let prefix_width: usize = 2; // "⏺ " = 2 display columns
            let wrap_width = (width as usize).saturating_sub(prefix_width).max(1);

            let md_lines = render_markdown(text);
            // Wrap all markdown lines to fit within the content area.
            let wrapped_md = wrap_md_lines(md_lines, wrap_width, prefix_width);

            for (i, md_line) in wrapped_md.iter().enumerate() {
                let prefix = if i == 0 {
                    Span::styled(
                        format!("{} ", BLACK_CIRCLE),
                        Style::default().fg(theme.text),
                    )
                } else {
                    Span::raw("  ".to_string())
                };

                let mut spans = vec![prefix];
                spans.extend(md_line.spans.clone());
                lines.push(Line::from(spans));
            }
        }
        MessageEntry::ToolUse {
            name,
            input_summary,
            tool_use_id: _,
        } => {
            // Original: colored circle + bold tool name + (summary)
            // Circle color: yellow/orange while executing (shown before result arrives)
            let circle_color = Color::Rgb(215, 119, 87); // Claude orange for in-progress
            let prefix_width: usize = 2; // "⏺ "
            let content_width = (width as usize).saturating_sub(prefix_width).max(1);

            let summary_text = if input_summary.is_empty() {
                name.clone()
            } else {
                format!("{} ({})", name, input_summary)
            };

            // Wrap the summary if it's too long.
            let wrapped = word_wrap(&summary_text, content_width);
            for (i, wline) in wrapped.iter().enumerate() {
                let pfx = if i == 0 {
                    Span::styled(
                        format!("{} ", BLACK_CIRCLE),
                        Style::default().fg(circle_color),
                    )
                } else {
                    Span::raw("  ".to_string())
                };
                // The first wrapped line has the bold name; continuation is inactive.
                if i == 0 {
                    // Split into name part and rest.
                    if wline.len() > name.len() && wline.starts_with(name.as_str()) {
                        lines.push(Line::from(vec![
                            pfx,
                            Span::styled(
                                name.clone(),
                                Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(
                                wline[name.len()..].to_string(),
                                Style::default().fg(theme.inactive),
                            ),
                        ]));
                    } else {
                        lines.push(Line::from(vec![
                            pfx,
                            Span::styled(
                                wline.clone(),
                                Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
                            ),
                        ]));
                    }
                } else {
                    lines.push(Line::from(vec![
                        pfx,
                        Span::styled(wline.clone(), Style::default().fg(theme.inactive)),
                    ]));
                }
            }
        }
        MessageEntry::ToolResult {
            name: _,
            output,
            is_error,
            tool_use_id: _,
        } => {
            let w = width as usize;
            // Tool results show with "  ⎿  " prefix.
            // The ToolUse entry above already has the colored circle.
            if *is_error {
                // Error: show error indicator with ⎿ prefix, word-wrapped.
                let error_width = w.saturating_sub(TOOL_RESULT_PREFIX_WIDTH).max(10);
                let dim_style = Style::default()
                    .fg(theme.inactive)
                    .add_modifier(Modifier::DIM);
                let prefix_span = Span::styled(TOOL_RESULT_PREFIX.to_string(), dim_style);
                let cont_prefix = Span::styled("     ".to_string(), dim_style);

                for (line_idx, err_line) in output.lines().take(6).enumerate() {
                    let wrapped = word_wrap(err_line, error_width);
                    for (j, wl) in wrapped.iter().enumerate() {
                        let pfx = if line_idx == 0 && j == 0 {
                            prefix_span.clone()
                        } else {
                            cont_prefix.clone()
                        };
                        lines.push(Line::from(vec![
                            pfx,
                            Span::styled(wl.clone(), Style::default().fg(theme.error)),
                        ]));
                    }
                }
            } else if looks_like_diff(output) {
                let diff_lines =
                    diff_view::diff_to_lines(output, width.saturating_sub(TOOL_RESULT_PREFIX_WIDTH as u16));
                for dl in diff_lines {
                    let mut spans = vec![Span::styled(
                        TOOL_RESULT_PREFIX.to_string(),
                        Style::default()
                            .fg(theme.inactive)
                            .add_modifier(Modifier::DIM),
                    )];
                    spans.extend(dl.spans);
                    lines.push(Line::from(spans));
                }
            } else {
                let max_preview_lines = 6;
                for line in output.lines().take(max_preview_lines) {
                    lines.extend(render_tool_output_lines(line, w, theme));
                }
                let output_line_count = output.lines().count();
                if output_line_count > max_preview_lines {
                    lines.push(Line::from(Span::styled(
                        format!(
                            "{}... ({} more lines)",
                            TOOL_RESULT_PREFIX,
                            output_line_count - max_preview_lines
                        ),
                        Style::default()
                            .fg(theme.inactive)
                            .add_modifier(Modifier::DIM),
                    )));
                }
            }
        }
        MessageEntry::Thinking { text } => {
            // Original: "∴ Thinking" in dimColor + italic.
            // When expanded (verbose/show_thinking): thinking text in dim markdown, paddingLeft=2.
            // When collapsed: "∴ Thinking (ctrl+o to expand)" in dim+italic.
            if show_thinking {
                lines.push(Line::from(vec![Span::styled(
                    "\u{2234} Thinking\u{2026}".to_string(),
                    Style::default()
                        .fg(theme.thinking)
                        .add_modifier(Modifier::DIM | Modifier::ITALIC),
                )]));
                // Content indented by 2 (paddingLeft=2 in original)
                let think_width = (width as usize).saturating_sub(2).max(1);
                for line in text.lines() {
                    for wrapped in word_wrap(line, think_width) {
                        lines.push(Line::from(vec![
                            Span::raw("  "),
                            Span::styled(
                                wrapped,
                                Style::default()
                                    .fg(theme.thinking)
                                    .add_modifier(Modifier::DIM),
                            ),
                        ]));
                    }
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
            // System messages: plain text in inactive/muted color, word-wrapped.
            let sys_width = (width as usize).max(1);
            for line_text in text.split('\n') {
                for wrapped in word_wrap(line_text, sys_width) {
                    lines.push(Line::from(vec![Span::styled(
                        wrapped,
                        Style::default().fg(theme.inactive),
                    )]));
                }
            }
        }
    }

    lines
}

impl<'a> Widget for MessageListWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 {
            return;
        }

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
            self.list
                .scroll_offset
                .min(total_lines.saturating_sub(visible_height))
        };

        let end = total_lines.min(scroll + visible_height);
        let visible = &all_lines[scroll..end];

        // Pin content to bottom: if fewer lines than area, offset from top
        // so messages hug the prompt bar (matching TS behavior).
        let top_offset = if visible.len() < visible_height {
            (visible_height - visible.len()) as u16
        } else {
            0
        };

        for (i, line) in visible.iter().enumerate() {
            let y = area.y + top_offset + i as u16;
            if y >= area.y + area.height {
                break;
            }
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
            &MessageEntry::User {
                text: "hello".to_string(),
            },
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
            &MessageEntry::Assistant {
                text: "hi".to_string(),
            },
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
                tool_use_id: "test".to_string(),
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
            &MessageEntry::Thinking {
                text: "let me think...".to_string(),
            },
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
            &MessageEntry::Thinking {
                text: "let me think about this".to_string(),
            },
            80,
            true, // expanded
            &theme,
        );
        let text: String = lines.iter().map(|l| l.to_string()).collect();
        assert!(text.contains("Thinking"));
        assert!(text.contains("let me think about this"));
    }

    #[test]
    fn word_wrap_short_text_stays_single_line() {
        let result = word_wrap("hello world", 80);
        assert_eq!(result, vec!["hello world"]);
    }

    #[test]
    fn word_wrap_long_text_wraps_at_boundary() {
        // 30 cols: "This is a long sentence that" wraps
        let result = word_wrap("This is a long sentence that should wrap at word boundaries", 30);
        assert!(result.len() >= 2);
        for line in &result {
            assert!(
                UnicodeWidthStr::width(line.as_str()) <= 30,
                "Line '{}' exceeds 30 cols (got {})",
                line,
                UnicodeWidthStr::width(line.as_str())
            );
        }
    }

    #[test]
    fn assistant_message_wraps_at_width() {
        let theme = crate::theme::dark_theme();
        let long_text = "This is a very long assistant message that should definitely wrap when the terminal width is narrow, like 40 columns wide.";
        let lines = render_message(
            &MessageEntry::Assistant {
                text: long_text.to_string(),
            },
            40,
            false,
            &theme,
        );
        // Skip blank line at index 0; all content lines should fit in 40 cols.
        for (i, line) in lines.iter().enumerate().skip(1) {
            let text = line.to_string();
            let w = UnicodeWidthStr::width(text.as_str());
            assert!(
                w <= 40,
                "Assistant line {} '{}' exceeds 40 cols (got {})",
                i, text, w
            );
        }
    }

    #[test]
    fn user_message_wraps_at_width() {
        let theme = crate::theme::dark_theme();
        let long_text = "Please explain what this project does in detail with lots of examples and code snippets showing usage.";
        let lines = render_message(
            &MessageEntry::User {
                text: long_text.to_string(),
            },
            50,
            false,
            &theme,
        );
        for (i, line) in lines.iter().enumerate().skip(1) {
            let text = line.to_string();
            let w = UnicodeWidthStr::width(text.as_str());
            assert!(
                w <= 50,
                "User line {} '{}' exceeds 50 cols (got {})",
                i, text, w
            );
        }
    }

    #[test]
    fn tool_result_wraps_at_width() {
        let theme = crate::theme::dark_theme();
        let long_output = "This is a very long tool output line that contains a lot of text and should definitely wrap properly within the terminal.";
        let lines = render_message(
            &MessageEntry::ToolResult {
                name: "Read".to_string(),
                output: long_output.to_string(),
                is_error: false,
                tool_use_id: "test".to_string(),
            },
            50,
            false,
            &theme,
        );
        for (i, line) in lines.iter().enumerate() {
            let text = line.to_string();
            let w = UnicodeWidthStr::width(text.as_str());
            assert!(
                w <= 50,
                "Tool result line {} '{}' exceeds 50 cols (got {})",
                i, text, w
            );
        }
    }

    #[test]
    fn system_message_wraps_at_width() {
        let theme = crate::theme::dark_theme();
        let long_text = "A system notification that is very long and needs to be wrapped properly when displayed in a narrow terminal window.";
        let lines = render_message(
            &MessageEntry::System {
                text: long_text.to_string(),
            },
            40,
            false,
            &theme,
        );
        for (i, line) in lines.iter().enumerate() {
            let text = line.to_string();
            let w = UnicodeWidthStr::width(text.as_str());
            assert!(
                w <= 40,
                "System line {} '{}' exceeds 40 cols (got {})",
                i, text, w
            );
        }
    }

    #[test]
    fn wrap_spans_preserves_style() {
        let styled_line = Line::from(vec![
            Span::styled("Hello ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw("world this is a long text that needs wrapping"),
        ]);
        let result = wrap_spans(&styled_line, 20, 0);
        assert!(result.len() >= 2, "Expected wrapping, got {} lines", result.len());
        // First line's first span should be bold
        assert!(result[0].spans[0].style.add_modifier == Modifier::BOLD
                || result[0].spans.iter().any(|s| s.style.add_modifier == Modifier::BOLD));
    }
}
