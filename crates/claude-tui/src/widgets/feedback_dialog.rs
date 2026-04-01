use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Widget};
use std::path::PathBuf;

/// The user's feedback response.
#[derive(Clone, Debug, PartialEq)]
pub enum FeedbackResponse {
    /// Thumbs up (good experience).
    Good,
    /// Thumbs down (bad experience).
    Bad,
    /// Neutral / mixed experience.
    Neutral,
}

impl FeedbackResponse {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Good => "Good",
            Self::Bad => "Bad",
            Self::Neutral => "Neutral",
        }
    }

    pub fn symbol(&self) -> &'static str {
        match self {
            Self::Good => "+",
            Self::Bad => "-",
            Self::Neutral => "~",
        }
    }
}

/// Feedback entry stored on disk.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct FeedbackEntry {
    /// When the feedback was given (ISO 8601).
    pub timestamp: String,
    /// Rating text.
    pub rating: String,
    /// Free-form user comment.
    pub comment: String,
    /// Session ID.
    pub session_id: String,
    /// Model used.
    pub model: String,
    /// Number of messages in the session.
    pub message_count: usize,
}

/// States of the feedback dialog.
#[derive(Clone, Debug, PartialEq)]
pub enum FeedbackState {
    /// Dialog not shown.
    Closed,
    /// Asking for a rating (1=Good, 2=Bad, 3=Neutral).
    AskRating,
    /// Asking for optional free-form feedback.
    AskComment,
    /// Showing thank-you message.
    Thanks,
}

/// State for the feedback dialog.
#[derive(Clone, Debug)]
pub struct FeedbackDialog {
    /// Current dialog state.
    pub state: FeedbackState,
    /// Selected response (set after AskRating).
    pub response: Option<FeedbackResponse>,
    /// Free-form comment text.
    pub comment: String,
    /// Cursor position within comment.
    pub cursor: usize,
    /// Session context for storage.
    pub session_id: String,
    /// Model name.
    pub model: String,
    /// Message count.
    pub message_count: usize,
}

impl FeedbackDialog {
    pub fn new(session_id: &str, model: &str, message_count: usize) -> Self {
        Self {
            state: FeedbackState::Closed,
            response: None,
            comment: String::new(),
            cursor: 0,
            session_id: session_id.to_string(),
            model: model.to_string(),
            message_count,
        }
    }

    /// Show the dialog (typically after session ends).
    pub fn open(&mut self) {
        self.state = FeedbackState::AskRating;
        self.response = None;
        self.comment.clear();
        self.cursor = 0;
    }

    /// Whether the dialog is currently visible.
    pub fn is_open(&self) -> bool {
        self.state != FeedbackState::Closed
    }

    /// Handle a key event.  Returns `true` when the dialog should be dismissed.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        match self.state {
            FeedbackState::Closed => false,
            FeedbackState::AskRating => self.handle_rating_key(key),
            FeedbackState::AskComment => self.handle_comment_key(key),
            FeedbackState::Thanks => {
                // Any key dismisses the thanks message
                self.state = FeedbackState::Closed;
                true
            }
        }
    }

    fn handle_rating_key(&mut self, key: KeyEvent) -> bool {
        match (key.modifiers, key.code) {
            (_, KeyCode::Char('1')) => {
                self.response = Some(FeedbackResponse::Good);
                self.state = FeedbackState::AskComment;
                false
            }
            (_, KeyCode::Char('2')) => {
                self.response = Some(FeedbackResponse::Bad);
                self.state = FeedbackState::AskComment;
                false
            }
            (_, KeyCode::Char('3')) => {
                self.response = Some(FeedbackResponse::Neutral);
                self.state = FeedbackState::AskComment;
                false
            }
            (_, KeyCode::Esc) | (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
                self.state = FeedbackState::Closed;
                true
            }
            _ => false,
        }
    }

    fn handle_comment_key(&mut self, key: KeyEvent) -> bool {
        match (key.modifiers, key.code) {
            (_, KeyCode::Enter) => {
                // Save feedback and show thanks
                self.save_feedback();
                self.state = FeedbackState::Thanks;
                false
            }
            (_, KeyCode::Esc) => {
                // Skip comment, still save with empty comment
                self.save_feedback();
                self.state = FeedbackState::Thanks;
                false
            }
            (_, KeyCode::Char(c))
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.comment.insert(self.cursor, c);
                self.cursor += c.len_utf8();
                false
            }
            (_, KeyCode::Backspace) => {
                if self.cursor > 0 {
                    let prev = self.comment[..self.cursor]
                        .chars()
                        .last()
                        .map(|c| c.len_utf8())
                        .unwrap_or(0);
                    self.cursor -= prev;
                    self.comment.remove(self.cursor);
                }
                false
            }
            (_, KeyCode::Left) => {
                if self.cursor > 0 {
                    let prev = self.comment[..self.cursor]
                        .chars()
                        .last()
                        .map(|c| c.len_utf8())
                        .unwrap_or(0);
                    self.cursor -= prev;
                }
                false
            }
            (_, KeyCode::Right) => {
                if self.cursor < self.comment.len() {
                    let next = self.comment[self.cursor..]
                        .chars()
                        .next()
                        .map(|c| c.len_utf8())
                        .unwrap_or(0);
                    self.cursor += next;
                }
                false
            }
            _ => false,
        }
    }

    /// Save the feedback entry to disk.
    fn save_feedback(&self) {
        let entry = FeedbackEntry {
            timestamp: chrono::Utc::now().to_rfc3339(),
            rating: self
                .response
                .as_ref()
                .map(|r| r.label().to_string())
                .unwrap_or_default(),
            comment: self.comment.clone(),
            session_id: self.session_id.clone(),
            model: self.model.clone(),
            message_count: self.message_count,
        };

        if let Some(path) = Self::feedback_file_path() {
            // Ensure parent directory exists
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            // Append as JSONL
            let line = match serde_json::to_string(&entry) {
                Ok(json) => format!("{}\n", json),
                Err(_) => return,
            };
            let _ = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()));
        }
    }

    /// Path to the feedback JSONL file.
    pub fn feedback_file_path() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(".claude").join("feedback.jsonl"))
    }

    /// Load all stored feedback entries.
    pub fn load_feedback() -> Vec<FeedbackEntry> {
        let path = match Self::feedback_file_path() {
            Some(p) => p,
            None => return Vec::new(),
        };
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        content
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }
}

/// Widget that renders the feedback dialog in the TUI.
pub struct FeedbackWidget<'a> {
    pub dialog: &'a FeedbackDialog,
}

impl<'a> FeedbackWidget<'a> {
    pub fn new(dialog: &'a FeedbackDialog) -> Self {
        Self { dialog }
    }
}

impl Widget for FeedbackWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if !self.dialog.is_open() {
            return;
        }

        Clear.render(area, buf);

        let block = Block::default()
            .title(" Session Feedback ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Magenta));

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height < 3 {
            return;
        }

        match self.dialog.state {
            FeedbackState::AskRating => {
                let question = Line::from(Span::styled(
                    "How was this session?",
                    Style::default().add_modifier(Modifier::BOLD),
                ));
                buf.set_line(inner.x + 1, inner.y, &question, inner.width.saturating_sub(2));

                if inner.height > 2 {
                    let options = Line::from(vec![
                        Span::styled("[1] ", Style::default().fg(Color::DarkGray)),
                        Span::styled("Good  ", Style::default().fg(Color::Green)),
                        Span::styled("[2] ", Style::default().fg(Color::DarkGray)),
                        Span::styled("Bad  ", Style::default().fg(Color::Red)),
                        Span::styled("[3] ", Style::default().fg(Color::DarkGray)),
                        Span::styled("Neutral", Style::default().fg(Color::Yellow)),
                    ]);
                    buf.set_line(
                        inner.x + 1,
                        inner.y + 2,
                        &options,
                        inner.width.saturating_sub(2),
                    );
                }

                if inner.height > 4 {
                    let hint = Line::from(Span::styled(
                        "Press 1-3 to rate, Esc to skip",
                        Style::default().fg(Color::DarkGray),
                    ));
                    buf.set_line(
                        inner.x + 1,
                        inner.y + inner.height - 1,
                        &hint,
                        inner.width.saturating_sub(2),
                    );
                }
            }

            FeedbackState::AskComment => {
                let rating_label = self
                    .dialog
                    .response
                    .as_ref()
                    .map(|r| r.label())
                    .unwrap_or("?");
                let header = Line::from(vec![
                    Span::raw("Rated: "),
                    Span::styled(
                        rating_label,
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                ]);
                buf.set_line(inner.x + 1, inner.y, &header, inner.width.saturating_sub(2));

                if inner.height > 2 {
                    let prompt_text = Line::from(Span::raw(
                        "Any additional feedback? (Enter to submit, Esc to skip)",
                    ));
                    buf.set_line(
                        inner.x + 1,
                        inner.y + 1,
                        &prompt_text,
                        inner.width.saturating_sub(2),
                    );
                }

                if inner.height > 3 {
                    let input_line = Line::from(vec![
                        Span::styled("> ", Style::default().fg(Color::Magenta)),
                        Span::raw(self.dialog.comment.clone()),
                    ]);
                    buf.set_line(
                        inner.x + 1,
                        inner.y + 3,
                        &input_line,
                        inner.width.saturating_sub(2),
                    );
                }
            }

            FeedbackState::Thanks => {
                let thanks = Line::from(Span::styled(
                    "Thanks for your feedback!",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ));
                buf.set_line(inner.x + 1, inner.y, &thanks, inner.width.saturating_sub(2));

                if inner.height > 2 {
                    let dismiss = Line::from(Span::styled(
                        "Press any key to continue",
                        Style::default().fg(Color::DarkGray),
                    ));
                    buf.set_line(
                        inner.x + 1,
                        inner.y + 2,
                        &dismiss,
                        inner.width.saturating_sub(2),
                    );
                }
            }

            FeedbackState::Closed => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_dialog() -> FeedbackDialog {
        FeedbackDialog::new("test-session-123", "claude-sonnet-4-6", 15)
    }

    #[test]
    fn test_initially_closed() {
        let d = make_dialog();
        assert!(!d.is_open());
        assert_eq!(d.state, FeedbackState::Closed);
    }

    #[test]
    fn test_open_sets_ask_rating() {
        let mut d = make_dialog();
        d.open();
        assert!(d.is_open());
        assert_eq!(d.state, FeedbackState::AskRating);
    }

    #[test]
    fn test_rating_good_transitions_to_comment() {
        let mut d = make_dialog();
        d.open();
        let key = KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE);
        d.handle_key(key);
        assert_eq!(d.state, FeedbackState::AskComment);
        assert_eq!(d.response, Some(FeedbackResponse::Good));
    }

    #[test]
    fn test_rating_bad_transitions_to_comment() {
        let mut d = make_dialog();
        d.open();
        let key = KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE);
        d.handle_key(key);
        assert_eq!(d.state, FeedbackState::AskComment);
        assert_eq!(d.response, Some(FeedbackResponse::Bad));
    }

    #[test]
    fn test_rating_neutral_transitions_to_comment() {
        let mut d = make_dialog();
        d.open();
        let key = KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE);
        d.handle_key(key);
        assert_eq!(d.state, FeedbackState::AskComment);
        assert_eq!(d.response, Some(FeedbackResponse::Neutral));
    }

    #[test]
    fn test_esc_during_rating_closes() {
        let mut d = make_dialog();
        d.open();
        let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        let dismissed = d.handle_key(esc);
        assert!(dismissed);
        assert!(!d.is_open());
    }

    #[test]
    fn test_comment_input() {
        let mut d = make_dialog();
        d.open();
        // Select rating
        d.handle_key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE));
        // Type comment
        d.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE));
        d.handle_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
        assert_eq!(d.comment, "hi");
    }

    #[test]
    fn test_comment_backspace() {
        let mut d = make_dialog();
        d.open();
        d.handle_key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE));
        d.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        d.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE));
        d.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(d.comment, "a");
    }

    #[test]
    fn test_enter_during_comment_shows_thanks() {
        let mut d = make_dialog();
        d.open();
        d.handle_key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE));
        d.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(d.state, FeedbackState::Thanks);
    }

    #[test]
    fn test_thanks_any_key_dismisses() {
        let mut d = make_dialog();
        d.state = FeedbackState::Thanks;
        let dismissed = d.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        assert!(dismissed);
        assert!(!d.is_open());
    }

    #[test]
    fn test_feedback_entry_serialization() {
        let entry = FeedbackEntry {
            timestamp: "2026-03-31T00:00:00Z".into(),
            rating: "Good".into(),
            comment: "Great session".into(),
            session_id: "abc-123".into(),
            model: "test".into(),
            message_count: 10,
        };
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: FeedbackEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.rating, "Good");
        assert_eq!(parsed.comment, "Great session");
    }

    #[test]
    fn test_widget_renders_rating_state() {
        let mut d = make_dialog();
        d.open();
        let widget = FeedbackWidget::new(&d);
        let area = Rect::new(0, 0, 50, 10);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);
        let content: String = buf
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(
            content.contains("session") || content.contains("Feedback") || content.contains("Good"),
            "Buffer should contain feedback prompt content"
        );
    }

    #[test]
    fn test_widget_hidden_when_closed() {
        let d = make_dialog();
        let widget = FeedbackWidget::new(&d);
        let area = Rect::new(0, 0, 50, 10);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);
        let content: String = buf.content().iter().map(|c| c.symbol().to_string()).collect();
        assert!(
            content.trim().is_empty(),
            "Closed dialog should not render"
        );
    }
}
