use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

/// The prompt character matching the original Claude Code: ❯ (figures.pointer).
const PROMPT_CHAR: &str = "\u{276F}";

pub enum InputAction {
    Submit(String),
    None,
}

pub struct PromptInput {
    text: String,
    cursor: usize, // byte position
    history: Vec<String>,
    history_index: Option<usize>, // None = current input, Some(i) = history[i]
    saved_current: String,        // Current input saved when browsing history
}

impl PromptInput {
    pub fn new() -> Self {
        Self {
            text: String::new(),
            cursor: 0,
            history: Vec::new(),
            history_index: None,
            saved_current: String::new(),
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> InputAction {
        match (key.modifiers, key.code) {
            // Shift+Enter or Alt+Enter: insert newline at cursor (multi-line input)
            (KeyModifiers::SHIFT, KeyCode::Enter) | (KeyModifiers::ALT, KeyCode::Enter) => {
                self.text.insert(self.cursor, '\n');
                self.cursor += '\n'.len_utf8();
                InputAction::None
            },
            // Submit on Enter
            (_, KeyCode::Enter) if !self.text.is_empty() => {
                let submitted = self.text.clone();
                self.history.push(submitted.clone());
                self.text.clear();
                self.cursor = 0;
                self.history_index = None;
                InputAction::Submit(submitted)
            },
            // Character input
            (_, KeyCode::Char(c))
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.text.insert(self.cursor, c);
                self.cursor += c.len_utf8();
                InputAction::None
            },
            // Backspace
            (_, KeyCode::Backspace) => {
                if self.cursor > 0 {
                    let prev = self.text[..self.cursor]
                        .chars()
                        .last()
                        .map(|c| c.len_utf8())
                        .unwrap_or(0);
                    self.cursor -= prev;
                    self.text.remove(self.cursor);
                }
                InputAction::None
            },
            // Delete
            (_, KeyCode::Delete) => {
                if self.cursor < self.text.len() {
                    self.text.remove(self.cursor);
                }
                InputAction::None
            },
            // Ctrl+Left — move cursor one word left (must be before catch-all Left)
            (KeyModifiers::CONTROL, KeyCode::Left) => {
                self.cursor = self.word_boundary_left();
                InputAction::None
            },
            // Ctrl+Right — move cursor one word right (must be before catch-all Right)
            (KeyModifiers::CONTROL, KeyCode::Right) => {
                self.cursor = self.word_boundary_right();
                InputAction::None
            },
            // Left arrow
            (_, KeyCode::Left) => {
                if self.cursor > 0 {
                    let prev = self.text[..self.cursor]
                        .chars()
                        .last()
                        .map(|c| c.len_utf8())
                        .unwrap_or(0);
                    self.cursor -= prev;
                }
                InputAction::None
            },
            // Right arrow
            (_, KeyCode::Right) => {
                if self.cursor < self.text.len() {
                    let next = self.text[self.cursor..]
                        .chars()
                        .next()
                        .map(|c| c.len_utf8())
                        .unwrap_or(0);
                    self.cursor += next;
                }
                InputAction::None
            },
            // Ctrl+A — home
            (KeyModifiers::CONTROL, KeyCode::Char('a')) => {
                self.cursor = 0;
                InputAction::None
            },
            // Ctrl+E — end
            (KeyModifiers::CONTROL, KeyCode::Char('e')) => {
                self.cursor = self.text.len();
                InputAction::None
            },
            // Ctrl+K — kill to end of line
            (KeyModifiers::CONTROL, KeyCode::Char('k')) => {
                self.text.truncate(self.cursor);
                InputAction::None
            },
            // Ctrl+U — kill to start of line
            (KeyModifiers::CONTROL, KeyCode::Char('u')) => {
                self.text = self.text[self.cursor..].to_string();
                self.cursor = 0;
                InputAction::None
            },
            // Ctrl+W — delete word before cursor
            (KeyModifiers::CONTROL, KeyCode::Char('w')) => {
                let new_pos = self.word_boundary_left();
                self.text = format!("{}{}", &self.text[..new_pos], &self.text[self.cursor..]);
                self.cursor = new_pos;
                InputAction::None
            },
            // Up — history previous
            (_, KeyCode::Up) => {
                self.history_prev();
                InputAction::None
            },
            // Down — history next
            (_, KeyCode::Down) => {
                self.history_next();
                InputAction::None
            },
            // Home
            (_, KeyCode::Home) => {
                self.cursor = 0;
                InputAction::None
            },
            // End
            (_, KeyCode::End) => {
                self.cursor = self.text.len();
                InputAction::None
            },
            _ => InputAction::None,
        }
    }

    /// Find the byte position of the start of the previous word.
    fn word_boundary_left(&self) -> usize {
        let before = &self.text[..self.cursor];
        // Skip trailing whitespace, then skip word chars
        let trimmed = before.trim_end();
        if trimmed.is_empty() {
            return 0;
        }
        // Find last whitespace in trimmed portion
        match trimmed.rfind(|c: char| c.is_whitespace()) {
            Some(pos) => {
                // Move past the whitespace char
                pos + trimmed[pos..]
                    .chars()
                    .next()
                    .map(|c| c.len_utf8())
                    .unwrap_or(0)
            },
            None => 0,
        }
    }

    /// Find the byte position of the end of the next word.
    fn word_boundary_right(&self) -> usize {
        let after = &self.text[self.cursor..];
        // Skip leading whitespace, then skip word chars
        let mut chars = after.char_indices();
        // Skip whitespace
        let mut pos = 0;
        for (i, c) in &mut chars {
            if !c.is_whitespace() {
                pos = i;
                break;
            }
            pos = i + c.len_utf8();
        }
        // Skip word chars
        for (i, c) in after[pos..].char_indices() {
            if c.is_whitespace() {
                return self.cursor + pos + i;
            }
        }
        self.text.len()
    }

    fn history_prev(&mut self) {
        if self.history.is_empty() {
            return;
        }
        match self.history_index {
            None => {
                self.saved_current = self.text.clone();
                self.history_index = Some(self.history.len() - 1);
            },
            Some(0) => return, // Already at oldest
            Some(i) => {
                self.history_index = Some(i - 1);
            },
        }
        if let Some(i) = self.history_index {
            self.text = self.history[i].clone();
            self.cursor = self.text.len();
        }
    }

    fn history_next(&mut self) {
        match self.history_index {
            None => (),
            Some(i) if i >= self.history.len() - 1 => {
                self.history_index = None;
                self.text = self.saved_current.clone();
                self.cursor = self.text.len();
            },
            Some(i) => {
                self.history_index = Some(i + 1);
                self.text = self.history[i + 1].clone();
                self.cursor = self.text.len();
            },
        }
    }
}

impl Default for PromptInput {
    fn default() -> Self {
        Self::new()
    }
}

// Widget implementation for rendering
pub struct PromptInputWidget<'a> {
    input: &'a PromptInput,
    style: Style,
    is_loading: bool,
    border_color: Color,
}

impl<'a> PromptInputWidget<'a> {
    pub fn new(input: &'a PromptInput) -> Self {
        Self {
            input,
            style: Style::default(),
            is_loading: false,
            border_color: Color::Rgb(136, 136, 136), // promptBorder default
        }
    }

    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    pub fn loading(mut self, loading: bool) -> Self {
        self.is_loading = loading;
        self
    }

    pub fn border_color(mut self, color: Color) -> Self {
        self.border_color = color;
        self
    }
}

impl<'a> Widget for PromptInputWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 {
            return;
        }

        // The original uses borderStyle="round" with borderLeft=false, borderRight=false,
        // borderBottom, borderTop. This creates a rounded top border spanning the width,
        // then the prompt character and input text, then a rounded bottom border.
        //
        // Render: top rounded border, then prompt line, then bottom rounded border.
        let border_style = Style::default().fg(self.border_color);

        let prompt_style = if self.is_loading {
            Style::default()
                .fg(self.border_color)
                .add_modifier(Modifier::DIM)
        } else {
            Style::default()
        };

        // Split the text into visual lines for multi-line display.
        // The first line gets the prompt character; subsequent lines are indented.
        let text_lines: Vec<&str> = self.input.text.split('\n').collect();
        let text_line_count = text_lines.len().max(1);

        if area.height >= 2 {
            // Top border
            let top_border = "─".repeat(area.width as usize);
            buf.set_string(area.x, area.y, &top_border, border_style);

            // Render text lines — first line has prompt char, rest are indented
            let prompt_prefix = format!("{} ", PROMPT_CHAR);
            let indent_width = prompt_prefix.chars().count();
            let indent = " ".repeat(indent_width);
            // How many rows are available between the two borders
            let content_rows = (area.height as usize).saturating_sub(2).max(1);
            // Show only the last `content_rows` lines so the active line stays visible
            let start = text_line_count.saturating_sub(content_rows);
            for (i, text_slice) in text_lines[start..].iter().enumerate() {
                let row = area.y + 1 + i as u16;
                if row + 1 >= area.y + area.height {
                    break; // leave room for bottom border
                }
                if start + i == 0 {
                    let line = Line::from(vec![
                        Span::styled(prompt_prefix.clone(), prompt_style),
                        Span::raw(*text_slice),
                    ]);
                    buf.set_line(area.x, row, &line, area.width);
                } else {
                    let line = Line::from(vec![Span::raw(indent.clone()), Span::raw(*text_slice)]);
                    buf.set_line(area.x, row, &line, area.width);
                }
            }

            // Bottom border
            let bottom_border = "─".repeat(area.width as usize);
            buf.set_string(
                area.x,
                area.y + area.height - 1,
                &bottom_border,
                border_style,
            );
        } else {
            // Minimal (height == 1): just the last line of text with prompt character
            let last_line = text_lines.last().copied().unwrap_or("");
            let line = Line::from(vec![
                Span::styled(format!("{} ", PROMPT_CHAR), prompt_style),
                Span::raw(last_line),
            ]);
            buf.set_line(area.x, area.y, &line, area.width);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_char_is_pointer() {
        // The original uses figures.pointer which is ❯
        assert_eq!(PROMPT_CHAR, "\u{276F}");
        assert_eq!(PROMPT_CHAR, "❯");
    }

    #[test]
    fn prompt_input_basic() {
        let mut input = PromptInput::new();
        assert!(input.is_empty());

        input.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::empty()));
        input.handle_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::empty()));
        assert_eq!(input.text(), "hi");
    }
}
