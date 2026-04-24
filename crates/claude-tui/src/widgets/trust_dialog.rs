use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Widget};
use std::collections::HashSet;
use std::path::PathBuf;

/// A trust decision made by the user.
#[derive(Clone, Debug, PartialEq)]
pub enum TrustDecision {
    /// Allow for this session only.
    AllowOnce,
    /// Allow and remember (persist to disk).
    AllowAlways,
    /// Deny for this session.
    Deny,
}

/// What kind of entity is requesting trust.
#[derive(Clone, Debug, PartialEq)]
pub enum TrustSubject {
    /// An MCP server.
    McpServer { name: String, command: String },
    /// A project hook.
    Hook { event: String, source: String },
    /// A bash permission configuration from project settings.
    BashPermission { source: String },
}

impl TrustSubject {
    /// A unique key for remembering trust decisions.
    pub fn trust_key(&self) -> String {
        match self {
            Self::McpServer { name, .. } => format!("mcp:{}", name),
            Self::Hook { event, source } => format!("hook:{}:{}", event, source),
            Self::BashPermission { source } => format!("bash:{}", source),
        }
    }

    /// Human-readable label.
    pub fn label(&self) -> String {
        match self {
            Self::McpServer { name, .. } => format!("MCP Server: {}", name),
            Self::Hook { event, .. } => format!("Hook: {}", event),
            Self::BashPermission { source } => format!("Bash permission: {}", source),
        }
    }

    /// Detailed description for the dialog.
    pub fn description(&self) -> String {
        match self {
            Self::McpServer { name, command } => {
                format!(
                    "A project configuration wants to connect to MCP server \"{}\".\n\
                     Command: {}",
                    name, command
                )
            }
            Self::Hook { event, source } => {
                format!(
                    "A project configuration defines a hook for the \"{}\" event.\n\
                     Source: {}",
                    event, source
                )
            }
            Self::BashPermission { source } => {
                format!(
                    "A project configuration grants bash execution permissions.\n\
                     Source: {}",
                    source
                )
            }
        }
    }
}

/// State for the trust confirmation dialog.
#[derive(Clone, Debug)]
pub struct TrustDialog {
    /// The subject requesting trust.
    pub subject: Option<TrustSubject>,
    /// Currently selected button (0=Allow Once, 1=Allow Always, 2=Deny).
    pub selected_button: usize,
    /// Whether the dialog is active.
    pub active: bool,
    /// Set of trust keys that have been remembered as allowed.
    trusted_keys: HashSet<String>,
    /// Set of trust keys that have been remembered as denied.
    denied_keys: HashSet<String>,
}

impl TrustDialog {
    pub fn new() -> Self {
        let trusted_keys = Self::load_trusted_keys();
        Self {
            subject: None,
            selected_button: 0,
            active: false,
            trusted_keys,
            denied_keys: HashSet::new(),
        }
    }

    /// Check whether a subject has already been trusted.
    pub fn is_trusted(&self, subject: &TrustSubject) -> bool {
        self.trusted_keys.contains(&subject.trust_key())
    }

    /// Check whether a subject has been denied.
    pub fn is_denied(&self, subject: &TrustSubject) -> bool {
        self.denied_keys.contains(&subject.trust_key())
    }

    /// Show the trust dialog for the given subject.
    /// Returns `None` if the subject is already trusted (auto-approved).
    pub fn prompt(&mut self, subject: TrustSubject) -> Option<()> {
        if self.is_trusted(&subject) {
            return None; // already trusted
        }
        self.subject = Some(subject);
        self.selected_button = 0;
        self.active = true;
        Some(())
    }

    /// Whether the dialog is currently shown.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Navigate to the next button.
    pub fn next_button(&mut self) {
        self.selected_button = (self.selected_button + 1) % 3;
    }

    /// Navigate to the previous button.
    pub fn prev_button(&mut self) {
        self.selected_button = (self.selected_button + 2) % 3;
    }

    /// Handle a key event.  Returns `Some(decision)` when the user makes a choice.
    pub fn handle_key(&mut self, key: KeyEvent) -> Option<TrustDecision> {
        if !self.active {
            return None;
        }

        match (key.modifiers, key.code) {
            (_, KeyCode::Left) | (_, KeyCode::BackTab) => {
                self.prev_button();
                None
            }
            (_, KeyCode::Right) | (_, KeyCode::Tab) => {
                self.next_button();
                None
            }
            (_, KeyCode::Enter) => {
                let decision = match self.selected_button {
                    0 => TrustDecision::AllowOnce,
                    1 => TrustDecision::AllowAlways,
                    2 => TrustDecision::Deny,
                    _ => TrustDecision::AllowOnce,
                };
                self.apply_decision(&decision);
                self.active = false;
                Some(decision)
            }
            (_, KeyCode::Esc) | (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
                self.active = false;
                let decision = TrustDecision::Deny;
                self.apply_decision(&decision);
                Some(decision)
            }
            // Shortcut keys
            (_, KeyCode::Char('a')) => {
                let decision = TrustDecision::AllowOnce;
                self.apply_decision(&decision);
                self.active = false;
                Some(decision)
            }
            (_, KeyCode::Char('A')) => {
                let decision = TrustDecision::AllowAlways;
                self.apply_decision(&decision);
                self.active = false;
                Some(decision)
            }
            (_, KeyCode::Char('d')) => {
                let decision = TrustDecision::Deny;
                self.apply_decision(&decision);
                self.active = false;
                Some(decision)
            }
            _ => None,
        }
    }

    /// Apply a trust decision to internal state and optionally persist.
    fn apply_decision(&mut self, decision: &TrustDecision) {
        if let Some(ref subject) = self.subject {
            let key = subject.trust_key();
            match decision {
                TrustDecision::AllowOnce => {
                    // Session-only: just remember in the in-memory set
                    self.trusted_keys.insert(key);
                }
                TrustDecision::AllowAlways => {
                    self.trusted_keys.insert(key.clone());
                    Self::persist_trusted_key(&key);
                }
                TrustDecision::Deny => {
                    self.denied_keys.insert(key);
                }
            }
        }
    }

    /// Path to the trusted keys file.
    fn trusted_keys_path() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(".claude").join("trusted_servers.json"))
    }

    /// Load trusted keys from disk.
    fn load_trusted_keys() -> HashSet<String> {
        let path = match Self::trusted_keys_path() {
            Some(p) => p,
            None => return HashSet::new(),
        };
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => return HashSet::new(),
        };
        serde_json::from_str(&content).unwrap_or_default()
    }

    /// Persist a single trusted key to disk.
    fn persist_trusted_key(key: &str) {
        let path = match Self::trusted_keys_path() {
            Some(p) => p,
            None => return,
        };
        // Load existing, add new, save back
        let mut keys = Self::load_trusted_keys();
        keys.insert(key.to_string());
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(&keys) {
            let _ = std::fs::write(&path, json);
        }
    }
}

impl Default for TrustDialog {
    fn default() -> Self {
        Self::new()
    }
}

/// Widget that renders the trust confirmation dialog.
pub struct TrustDialogWidget<'a> {
    pub dialog: &'a TrustDialog,
}

impl<'a> TrustDialogWidget<'a> {
    pub fn new(dialog: &'a TrustDialog) -> Self {
        Self { dialog }
    }
}

impl Widget for TrustDialogWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if !self.dialog.is_active() {
            return;
        }

        Clear.render(area, buf);

        let subject_label = self
            .dialog
            .subject
            .as_ref()
            .map(|s| s.label())
            .unwrap_or_else(|| "Unknown".to_string());

        let title = format!(" Trust: {} ", subject_label);
        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow));

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height < 5 {
            return;
        }

        // Warning header
        let warning = Line::from(Span::styled(
            "Do you trust this project configuration?",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
        buf.set_line(
            inner.x + 1,
            inner.y,
            &warning,
            inner.width.saturating_sub(2),
        );

        // Description text
        if let Some(ref subject) = self.dialog.subject {
            let desc = subject.description();
            let max_w = inner.width.saturating_sub(2) as usize;
            for (offset, line_text) in desc.lines().enumerate() {
                let row = inner.y + 2 + offset as u16;
                if row >= inner.y + inner.height.saturating_sub(2) {
                    break;
                }
                let display = if line_text.len() > max_w {
                    format!("{}...", &line_text[..max_w.saturating_sub(3)])
                } else {
                    line_text.to_string()
                };
                let line = Line::from(Span::raw(display));
                buf.set_line(inner.x + 1, row, &line, inner.width.saturating_sub(2));
            }
        }

        // Buttons at bottom
        let button_y = inner.y + inner.height - 1;
        let buttons = [
            ("Allow Once (a)", 0),
            ("Allow Always (A)", 1),
            ("Deny (d)", 2),
        ];
        let mut x = inner.x + 2;
        for (label, idx) in &buttons {
            let style = if *idx == self.dialog.selected_button {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            let span = Span::styled(format!(" {} ", label), style);
            buf.set_span(x, button_y, &span, span.width() as u16);
            x += span.width() as u16 + 2;
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_subject() -> TrustSubject {
        TrustSubject::McpServer {
            name: "test-server".into(),
            command: "npx -y test-mcp-server".into(),
        }
    }

    #[test]
    fn test_trust_key_mcp() {
        let s = sample_subject();
        assert_eq!(s.trust_key(), "mcp:test-server");
    }

    #[test]
    fn test_trust_key_hook() {
        let s = TrustSubject::Hook {
            event: "PostToolUse".into(),
            source: ".claude/settings.json".into(),
        };
        assert_eq!(s.trust_key(), "hook:PostToolUse:.claude/settings.json");
    }

    #[test]
    fn test_prompt_shows_dialog() {
        let mut td = TrustDialog::new();
        let result = td.prompt(sample_subject());
        assert!(result.is_some());
        assert!(td.is_active());
    }

    #[test]
    fn test_prompt_skips_trusted() {
        let mut td = TrustDialog::new();
        td.trusted_keys.insert("mcp:test-server".into());
        let result = td.prompt(sample_subject());
        assert!(result.is_none());
        assert!(!td.is_active());
    }

    #[test]
    fn test_handle_key_enter_allow_once() {
        let mut td = TrustDialog::new();
        td.prompt(sample_subject());
        td.selected_button = 0;
        let decision = td.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(decision, Some(TrustDecision::AllowOnce));
        assert!(!td.is_active());
        assert!(td.is_trusted(&sample_subject()));
    }

    #[test]
    fn test_handle_key_enter_deny() {
        let mut td = TrustDialog::new();
        td.prompt(sample_subject());
        td.selected_button = 2;
        let decision = td.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(decision, Some(TrustDecision::Deny));
        assert!(!td.is_active());
        assert!(td.is_denied(&sample_subject()));
    }

    #[test]
    fn test_handle_key_shortcut_a() {
        let mut td = TrustDialog::new();
        td.prompt(sample_subject());
        let decision = td.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        assert_eq!(decision, Some(TrustDecision::AllowOnce));
    }

    #[test]
    fn test_handle_key_shortcut_d() {
        let mut td = TrustDialog::new();
        td.prompt(sample_subject());
        let decision = td.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
        assert_eq!(decision, Some(TrustDecision::Deny));
    }

    #[test]
    fn test_handle_key_esc_denies() {
        let mut td = TrustDialog::new();
        td.prompt(sample_subject());
        let decision = td.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(decision, Some(TrustDecision::Deny));
        assert!(!td.is_active());
    }

    #[test]
    fn test_button_navigation() {
        let mut td = TrustDialog::new();
        td.prompt(sample_subject());
        assert_eq!(td.selected_button, 0);
        td.next_button();
        assert_eq!(td.selected_button, 1);
        td.next_button();
        assert_eq!(td.selected_button, 2);
        td.next_button();
        assert_eq!(td.selected_button, 0);
        td.prev_button();
        assert_eq!(td.selected_button, 2);
    }

    #[test]
    fn test_widget_hidden_when_inactive() {
        let td = TrustDialog::new();
        let widget = TrustDialogWidget::new(&td);
        let area = Rect::new(0, 0, 60, 12);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);
        let content: String = buf
            .content()
            .iter()
            .map(|c| c.symbol().to_string())
            .collect();
        assert!(
            content.trim().is_empty(),
            "Inactive dialog should not render"
        );
    }

    #[test]
    fn test_widget_renders_when_active() {
        let mut td = TrustDialog::new();
        td.prompt(sample_subject());
        let widget = TrustDialogWidget::new(&td);
        let area = Rect::new(0, 0, 60, 12);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);
        let content: String = buf
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(
            content.contains("Trust") || content.contains("trust") || content.contains("Allow"),
            "Buffer should contain trust dialog content"
        );
    }

    #[test]
    fn test_subject_descriptions_not_empty() {
        let subjects = vec![
            sample_subject(),
            TrustSubject::Hook {
                event: "PreToolUse".into(),
                source: "project".into(),
            },
            TrustSubject::BashPermission {
                source: ".claude/settings.json".into(),
            },
        ];
        for s in &subjects {
            assert!(!s.description().is_empty());
            assert!(!s.label().is_empty());
        }
    }
}
