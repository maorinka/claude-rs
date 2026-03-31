use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Widget};

pub enum InputAction {
    Submit(String),
    None,
}

pub struct PromptInput {
    text: String,
    cursor: usize,          // byte position
    history: Vec<String>,
    history_index: Option<usize>, // None = current input, Some(i) = history[i]
    saved_current: String,  // Current input saved when browsing history
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

    pub fn handle_key(&mut self, key: KeyEvent) -> InputAction {
        match (key.modifiers, key.code) {
            // Submit on Enter
            (_, KeyCode::Enter) if !self.text.is_empty() => {
                let submitted = self.text.clone();
                self.history.push(submitted.clone());
                self.text.clear();
                self.cursor = 0;
                self.history_index = None;
                InputAction::Submit(submitted)
            }
            // Character input
            (_, KeyCode::Char(c))
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.text.insert(self.cursor, c);
                self.cursor += c.len_utf8();
                InputAction::None
            }
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
            }
            // Delete
            (_, KeyCode::Delete) => {
                if self.cursor < self.text.len() {
                    self.text.remove(self.cursor);
                }
                InputAction::None
            }
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
            }
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
            }
            // Ctrl+A — home
            (KeyModifiers::CONTROL, KeyCode::Char('a')) => {
                self.cursor = 0;
                InputAction::None
            }
            // Ctrl+E — end
            (KeyModifiers::CONTROL, KeyCode::Char('e')) => {
                self.cursor = self.text.len();
                InputAction::None
            }
            // Ctrl+K — kill to end of line
            (KeyModifiers::CONTROL, KeyCode::Char('k')) => {
                self.text.truncate(self.cursor);
                InputAction::None
            }
            // Ctrl+U — kill to start of line
            (KeyModifiers::CONTROL, KeyCode::Char('u')) => {
                self.text = self.text[self.cursor..].to_string();
                self.cursor = 0;
                InputAction::None
            }
            // Up — history previous
            (_, KeyCode::Up) => {
                self.history_prev();
                InputAction::None
            }
            // Down — history next
            (_, KeyCode::Down) => {
                self.history_next();
                InputAction::None
            }
            // Home
            (_, KeyCode::Home) => {
                self.cursor = 0;
                InputAction::None
            }
            // End
            (_, KeyCode::End) => {
                self.cursor = self.text.len();
                InputAction::None
            }
            _ => InputAction::None,
        }
    }

    fn history_prev(&mut self) {
        if self.history.is_empty() {
            return;
        }
        match self.history_index {
            None => {
                self.saved_current = self.text.clone();
                self.history_index = Some(self.history.len() - 1);
            }
            Some(0) => return, // Already at oldest
            Some(i) => {
                self.history_index = Some(i - 1);
            }
        }
        if let Some(i) = self.history_index {
            self.text = self.history[i].clone();
            self.cursor = self.text.len();
        }
    }

    fn history_next(&mut self) {
        match self.history_index {
            None => return,
            Some(i) if i >= self.history.len() - 1 => {
                self.history_index = None;
                self.text = self.saved_current.clone();
                self.cursor = self.text.len();
            }
            Some(i) => {
                self.history_index = Some(i + 1);
                self.text = self.history[i + 1].clone();
                self.cursor = self.text.len();
            }
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
}

impl<'a> PromptInputWidget<'a> {
    pub fn new(input: &'a PromptInput) -> Self {
        Self {
            input,
            style: Style::default(),
        }
    }

    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }
}

impl<'a> Widget for PromptInputWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 {
            return;
        }
        let text = &self.input.text;
        let prompt = "> ";
        let line = Line::from(vec![
            Span::styled(
                prompt,
                Style::default().fg(ratatui::style::Color::Cyan),
            ),
            Span::raw(text),
        ]);
        let block = Block::default().borders(Borders::TOP);
        let inner = block.inner(area);
        block.render(area, buf);
        buf.set_line(inner.x, inner.y, &line, inner.width);
    }
}
