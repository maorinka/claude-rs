use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Widget};

/// An inline text-input dialog shown when AskUserQuestionTool is awaiting a response.
pub struct AskUserDialog {
    /// The question the tool asked.
    pub question: String,
    /// Optional predefined choices.
    pub options: Vec<String>,
    /// The text the user is currently typing.
    pub input: String,
    /// Cursor byte offset within `input`.
    pub cursor: usize,
}

impl AskUserDialog {
    pub fn new(question: impl Into<String>, options: Vec<String>) -> Self {
        Self {
            question: question.into(),
            options,
            input: String::new(),
            cursor: 0,
        }
    }

    /// Handle a key event.  Returns `Some(answer)` when the user submits.
    pub fn handle_key(&mut self, key: KeyEvent) -> Option<String> {
        match (key.modifiers, key.code) {
            (_, KeyCode::Enter) => {
                if !self.input.is_empty() {
                    Some(self.input.clone())
                } else {
                    None
                }
            }
            (_, KeyCode::Char(c))
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.input.insert(self.cursor, c);
                self.cursor += c.len_utf8();
                None
            }
            (_, KeyCode::Backspace) => {
                if self.cursor > 0 {
                    let prev = self.input[..self.cursor]
                        .chars()
                        .last()
                        .map(|c| c.len_utf8())
                        .unwrap_or(0);
                    self.cursor -= prev;
                    self.input.remove(self.cursor);
                }
                None
            }
            (_, KeyCode::Left) => {
                if self.cursor > 0 {
                    let prev = self.input[..self.cursor]
                        .chars()
                        .last()
                        .map(|c| c.len_utf8())
                        .unwrap_or(0);
                    self.cursor -= prev;
                }
                None
            }
            (_, KeyCode::Right) => {
                if self.cursor < self.input.len() {
                    let next = self.input[self.cursor..]
                        .chars()
                        .next()
                        .map(|c| c.len_utf8())
                        .unwrap_or(0);
                    self.cursor += next;
                }
                None
            }
            _ => None,
        }
    }
}

impl Widget for &AskUserDialog {
    fn render(self, area: Rect, buf: &mut Buffer) {
        Clear.render(area, buf);

        let block = Block::default()
            .title(" Claude needs your input ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height < 3 {
            return;
        }

        // Question text (word-wrapped to fit width)
        let q_line = Line::from(Span::styled(
            self.question.clone(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ));
        buf.set_line(inner.x + 1, inner.y, &q_line, inner.width.saturating_sub(2));

        // Optional choices on the next line
        let mut row = inner.y + 1;
        if !self.options.is_empty() {
            let opts = self.options.join("  |  ");
            let opt_line = Line::from(Span::styled(opts, Style::default().fg(Color::DarkGray)));
            buf.set_line(inner.x + 1, row, &opt_line, inner.width.saturating_sub(2));
            row += 1;
        }

        // Input field at the bottom area
        if row < inner.y + inner.height {
            let prompt = "> ";
            let input_line = Line::from(vec![
                Span::styled(prompt, Style::default().fg(Color::Cyan)),
                Span::raw(self.input.clone()),
            ]);
            let input_y = inner.y + inner.height - 1;
            buf.set_line(
                inner.x + 1,
                input_y,
                &input_line,
                inner.width.saturating_sub(2),
            );
        }
    }
}
