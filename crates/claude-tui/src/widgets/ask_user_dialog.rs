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
    /// Currently highlighted preset option when options are present.
    selected_option: usize,
}

impl AskUserDialog {
    pub fn new(question: impl Into<String>, options: Vec<String>) -> Self {
        Self {
            question: question.into(),
            options,
            input: String::new(),
            cursor: 0,
            selected_option: 0,
        }
    }

    /// Handle a key event.  Returns `Some(answer)` when the user submits.
    pub fn handle_key(&mut self, key: KeyEvent) -> Option<String> {
        match (key.modifiers, key.code) {
            (_, KeyCode::Enter) => {
                if !self.input.is_empty() {
                    Some(self.input.clone())
                } else if !self.options.is_empty() {
                    self.options.get(self.selected_option).cloned()
                } else {
                    None
                }
            }
            (_, KeyCode::Tab) | (_, KeyCode::Down) | (_, KeyCode::Right) => {
                self.next_option();
                None
            }
            (_, KeyCode::BackTab) | (_, KeyCode::Up) | (_, KeyCode::Left) => {
                self.prev_option();
                None
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
            _ => None,
        }
    }

    fn next_option(&mut self) {
        if !self.options.is_empty() && self.input.is_empty() {
            self.selected_option = (self.selected_option + 1) % self.options.len();
        } else if self.cursor < self.input.len() {
            let next = self.input[self.cursor..]
                .chars()
                .next()
                .map(|c| c.len_utf8())
                .unwrap_or(0);
            self.cursor += next;
        }
    }

    fn prev_option(&mut self) {
        if !self.options.is_empty() && self.input.is_empty() {
            self.selected_option = self
                .selected_option
                .checked_sub(1)
                .unwrap_or(self.options.len() - 1);
        } else if self.cursor > 0 {
            let prev = self.input[..self.cursor]
                .chars()
                .last()
                .map(|c| c.len_utf8())
                .unwrap_or(0);
            self.cursor -= prev;
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
            let mut spans = Vec::new();
            for (idx, option) in self.options.iter().enumerate() {
                if idx > 0 {
                    spans.push(Span::styled("  ", Style::default().fg(Color::DarkGray)));
                }
                let selected = self.input.is_empty() && idx == self.selected_option;
                let style = if selected {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::DarkGray)
                };
                spans.push(Span::styled(format!(" {} ", option), style));
            }
            let opt_line = Line::from(spans);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn enter_selects_highlighted_option_when_input_empty() {
        let mut dialog = AskUserDialog::new("Pick one", vec!["yes".into(), "no".into()]);
        assert_eq!(dialog.handle_key(key(KeyCode::Enter)), Some("yes".into()));
    }

    #[test]
    fn tab_cycles_options() {
        let mut dialog = AskUserDialog::new("Pick one", vec!["yes".into(), "no".into()]);
        dialog.handle_key(key(KeyCode::Tab));
        assert_eq!(dialog.handle_key(key(KeyCode::Enter)), Some("no".into()));
    }

    #[test]
    fn typed_input_overrides_selected_option() {
        let mut dialog = AskUserDialog::new("Pick one", vec!["yes".into(), "no".into()]);
        dialog.handle_key(key(KeyCode::Char('m')));
        dialog.handle_key(key(KeyCode::Char('a')));
        dialog.handle_key(key(KeyCode::Char('y')));
        assert_eq!(dialog.handle_key(key(KeyCode::Enter)), Some("may".into()));
    }
}
