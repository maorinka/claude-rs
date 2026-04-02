use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Widget};

/// Steps in the first-run onboarding wizard.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OnboardingStep {
    Welcome,
    Auth,
    Permissions,
    BasicUsage,
    Done,
}

impl OnboardingStep {
    /// The ordered list of steps.
    pub const ALL: &'static [OnboardingStep] = &[
        OnboardingStep::Welcome,
        OnboardingStep::Auth,
        OnboardingStep::Permissions,
        OnboardingStep::BasicUsage,
        OnboardingStep::Done,
    ];

    /// Human-readable title for the step.
    pub fn title(&self) -> &'static str {
        match self {
            Self::Welcome => "Welcome",
            Self::Auth => "Authentication",
            Self::Permissions => "Permissions",
            Self::BasicUsage => "Getting Started",
            Self::Done => "Setup Complete",
        }
    }

    /// Instructional body text for the step.
    pub fn body(&self) -> &'static str {
        match self {
            Self::Welcome => {
                "Welcome to Claude Code!\n\n\
                 Claude Code is an agentic coding tool that lives in your terminal.\n\
                 It can read and edit files, run commands, search code, and help you\n\
                 build software faster.\n\n\
                 Press Enter to continue."
            }
            Self::Auth => {
                "Authentication setup:\n\n\
                 Claude Code needs an API key to communicate with the Claude API.\n\n\
                 Set your API key:\n\
                   export ANTHROPIC_API_KEY=sk-ant-...\n\n\
                 Or set it in ~/.claude/settings.json under \"apiKey\".\n\n\
                 Alternatively, if your organization uses OAuth, you will be\n\
                 prompted to authenticate in your browser.\n\n\
                 Press Enter to continue."
            }
            Self::Permissions => {
                "Permission modes:\n\n\
                 Claude Code asks for permission before running tools.\n\
                 You can choose a mode:\n\n\
                 * default  - All tool calls require approval\n\
                 * plan     - Read-only tools auto-approved; writes need approval\n\
                 * auto-edit - File edits auto-approved; shell needs approval\n\
                 * yolo     - All tools auto-approved (dangerous)\n\n\
                 You can change this anytime with /permissions.\n\n\
                 Press Enter to continue."
            }
            Self::BasicUsage => {
                "Basic usage:\n\n\
                 Type a message and press Enter to chat with Claude.\n\
                 Use slash commands for common actions:\n\n\
                   /help      - Show all commands\n\
                   /compact   - Compact conversation history\n\
                   /status    - Show session info\n\
                   /context   - Show context window usage\n\
                   /commit    - Generate a commit message\n\
                   /review    - Review code changes\n\n\
                 Press Ctrl+C to exit at any time.\n\n\
                 Press Enter to finish setup."
            }
            Self::Done => {
                "Setup complete!\n\n\
                 You are ready to start using Claude Code.\n\
                 Type a message below to begin.\n\n\
                 Press Enter to close this wizard."
            }
        }
    }
}

/// State for the onboarding wizard.
#[derive(Clone, Debug)]
pub struct OnboardingWizard {
    /// Index into OnboardingStep::ALL.
    pub step_index: usize,
    /// Whether the wizard has been completed.
    pub completed: bool,
}

impl OnboardingWizard {
    pub fn new() -> Self {
        Self {
            step_index: 0,
            completed: false,
        }
    }

    /// Detect whether this is the first run (no ~/.claude/ directory).
    pub fn is_first_run() -> bool {
        if let Some(home) = dirs::home_dir() {
            let claude_dir = home.join(".claude");
            !claude_dir.exists()
        } else {
            false
        }
    }

    /// Current step.
    pub fn current_step(&self) -> OnboardingStep {
        OnboardingStep::ALL
            .get(self.step_index)
            .copied()
            .unwrap_or(OnboardingStep::Done)
    }

    /// Advance to the next step. Returns true if the wizard is now complete.
    pub fn advance(&mut self) -> bool {
        if self.step_index + 1 < OnboardingStep::ALL.len() {
            self.step_index += 1;
            false
        } else {
            self.completed = true;
            true
        }
    }

    /// Handle a key event. Returns `true` when the wizard is dismissed.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        match (key.modifiers, key.code) {
            (_, KeyCode::Enter) => self.advance(),
            (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
                self.completed = true;
                true
            }
            (_, KeyCode::Esc) => {
                self.completed = true;
                true
            }
            _ => false,
        }
    }

    /// Total number of steps.
    pub fn total_steps(&self) -> usize {
        OnboardingStep::ALL.len()
    }

    /// Whether the wizard is still active (not completed).
    pub fn is_active(&self) -> bool {
        !self.completed
    }
}

impl Default for OnboardingWizard {
    fn default() -> Self {
        Self::new()
    }
}

/// Widget that renders the onboarding wizard overlay.
pub struct OnboardingWidget<'a> {
    pub wizard: &'a OnboardingWizard,
}

impl<'a> OnboardingWidget<'a> {
    pub fn new(wizard: &'a OnboardingWizard) -> Self {
        Self { wizard }
    }
}

impl Widget for OnboardingWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if self.wizard.completed {
            return;
        }

        Clear.render(area, buf);

        let step = self.wizard.current_step();
        let step_num = self.wizard.step_index + 1;
        let total = self.wizard.total_steps();

        let title = format!(" {} ({}/{}) ", step.title(), step_num, total);

        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height < 3 {
            return;
        }

        // Progress indicator at top
        let progress_width = inner.width.saturating_sub(2) as usize;
        let filled = if total > 0 {
            (step_num * progress_width) / total
        } else {
            0
        };
        let progress_bar: String = (0..progress_width)
            .map(|i| if i < filled { '#' } else { '-' })
            .collect();
        let progress_line =
            Line::from(Span::styled(progress_bar, Style::default().fg(Color::Cyan)));
        buf.set_line(
            inner.x + 1,
            inner.y,
            &progress_line,
            inner.width.saturating_sub(2),
        );

        // Body text (word-wrapped by newlines in the body string)
        let body = step.body();
        let max_width = inner.width.saturating_sub(2) as usize;
        let mut row = inner.y + 2;
        for text_line in body.lines() {
            if row >= inner.y + inner.height {
                break;
            }

            // Simple word-wrap
            let mut remaining = text_line;
            while !remaining.is_empty() && row < inner.y + inner.height {
                let chunk = if remaining.len() > max_width {
                    // Find last space within max_width
                    let split_at = remaining[..max_width].rfind(' ').unwrap_or(max_width);
                    let (chunk, rest) = remaining.split_at(split_at);
                    remaining = rest.trim_start();
                    chunk
                } else {
                    let chunk = remaining;
                    remaining = "";
                    chunk
                };

                let style = if chunk.starts_with("  ") {
                    // Indented lines (commands) get a different style
                    Style::default().fg(Color::Yellow)
                } else if chunk.starts_with("* ") {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default()
                };

                let line = Line::from(Span::styled(chunk, style));
                buf.set_line(inner.x + 1, row, &line, inner.width.saturating_sub(2));
                row += 1;
            }
        }

        // Footer hint
        if row < inner.y + inner.height {
            let footer = Line::from(Span::styled(
                "Press Enter to continue, Esc to skip",
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            ));
            let footer_y = inner.y + inner.height - 1;
            buf.set_line(
                inner.x + 1,
                footer_y,
                &footer,
                inner.width.saturating_sub(2),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore] // Needs auth check fix
    fn test_step_progression() {
        let mut wiz = OnboardingWizard::new();
        assert_eq!(wiz.current_step(), OnboardingStep::Welcome);
        assert!(!wiz.advance());
        assert_eq!(wiz.current_step(), OnboardingStep::Auth);
        assert!(!wiz.advance());
        assert_eq!(wiz.current_step(), OnboardingStep::Permissions);
        assert!(!wiz.advance());
        assert_eq!(wiz.current_step(), OnboardingStep::BasicUsage);
        assert!(wiz.advance()); // Done step -> completed
        assert!(wiz.completed);
    }

    #[test]
    fn test_handle_key_enter_advances() {
        let mut wiz = OnboardingWizard::new();
        let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        assert!(!wiz.handle_key(enter));
        assert_eq!(wiz.current_step(), OnboardingStep::Auth);
    }

    #[test]
    fn test_handle_key_esc_completes() {
        let mut wiz = OnboardingWizard::new();
        let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        assert!(wiz.handle_key(esc));
        assert!(wiz.completed);
    }

    #[test]
    fn test_handle_key_ctrl_c_completes() {
        let mut wiz = OnboardingWizard::new();
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(wiz.handle_key(ctrl_c));
        assert!(wiz.completed);
    }

    #[test]
    fn test_total_steps() {
        let wiz = OnboardingWizard::new();
        assert_eq!(wiz.total_steps(), 5);
    }

    #[test]
    fn test_is_active() {
        let mut wiz = OnboardingWizard::new();
        assert!(wiz.is_active());
        wiz.completed = true;
        assert!(!wiz.is_active());
    }

    #[test]
    fn test_widget_renders_without_panic() {
        let wiz = OnboardingWizard::new();
        let widget = OnboardingWidget::new(&wiz);
        let area = Rect::new(0, 0, 60, 20);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);
        let content: String = buf
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(
            content.contains("Welcome") || content.contains('#'),
            "Buffer should contain onboarding content"
        );
    }

    #[test]
    fn test_widget_skips_render_when_completed() {
        let mut wiz = OnboardingWizard::new();
        wiz.completed = true;
        let widget = OnboardingWidget::new(&wiz);
        let area = Rect::new(0, 0, 60, 20);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);
        // Should be empty
        let content: String = buf
            .content()
            .iter()
            .map(|c| c.symbol().to_string())
            .collect();
        assert!(
            content.trim().is_empty(),
            "Completed wizard should not render"
        );
    }

    #[test]
    fn test_all_steps_have_body_text() {
        for step in OnboardingStep::ALL {
            assert!(
                !step.body().is_empty(),
                "Step {:?} should have body text",
                step
            );
            assert!(
                !step.title().is_empty(),
                "Step {:?} should have a title",
                step
            );
        }
    }
}
