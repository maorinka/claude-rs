use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Widget};

use crate::theme::{preview_colors, ThemeSetting};

/// Interactive theme picker rendered inline above the prompt.
/// Matches the TS ThemePicker component: 7 options (Auto + 6 themes),
/// up/down to navigate, Enter to select, Esc to cancel.
pub struct ThemePicker {
    pub selected: usize,
    pub visible: bool,
    pub current_setting: ThemeSetting,
}

impl ThemePicker {
    pub fn new() -> Self {
        Self {
            selected: 0,
            visible: false,
            current_setting: ThemeSetting::Auto,
        }
    }

    /// Open the picker, highlighting the current theme setting.
    pub fn open(&mut self, current: ThemeSetting) {
        self.current_setting = current;
        self.visible = true;

        // Find the index of the current setting
        self.selected = ThemeSetting::ALL
            .iter()
            .position(|s| *s == current)
            .unwrap_or(0);
    }

    pub fn close(&mut self) {
        self.visible = false;
    }

    /// Move selection down.
    pub fn next(&mut self) {
        let len = ThemeSetting::ALL.len();
        if len > 0 {
            self.selected = (self.selected + 1) % len;
        }
    }

    /// Move selection up.
    pub fn prev(&mut self) {
        let len = ThemeSetting::ALL.len();
        if len > 0 {
            self.selected = self.selected.checked_sub(1).unwrap_or(len - 1);
        }
    }

    /// Confirm selection. Returns the chosen ThemeSetting.
    pub fn confirm(&mut self) -> Option<ThemeSetting> {
        let setting = ThemeSetting::ALL.get(self.selected).copied()?;
        self.close();
        Some(setting)
    }

    /// Currently highlighted setting (for live preview).
    pub fn highlighted_setting(&self) -> ThemeSetting {
        ThemeSetting::ALL
            .get(self.selected)
            .copied()
            .unwrap_or(ThemeSetting::Auto)
    }

    /// Height needed to render the picker (for layout).
    pub fn height(&self) -> u16 {
        // title row + options + blank + hint + border (2)
        (ThemeSetting::ALL.len() as u16) + 5
    }
}

impl Default for ThemePicker {
    fn default() -> Self {
        Self::new()
    }
}

/// Stateless widget that renders the theme picker inline.
pub struct ThemePickerWidget<'a> {
    picker: &'a ThemePicker,
}

impl<'a> ThemePickerWidget<'a> {
    pub fn new(picker: &'a ThemePicker) -> Self {
        Self { picker }
    }
}

impl<'a> Widget for ThemePickerWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if !self.picker.visible || area.height < 5 {
            return;
        }

        Clear.render(area, buf);

        let block = Block::default()
            .title(" Theme ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(ratatui::style::Color::Cyan));
        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height == 0 || inner.width == 0 {
            return;
        }

        let mut row: u16 = 0;

        // Title line
        if row < inner.height {
            let title = Line::from(Span::styled(
                "  Choose the text style that looks best with your terminal",
                Style::default().add_modifier(Modifier::BOLD),
            ));
            buf.set_line(inner.x, inner.y + row, &title, inner.width);
            row += 1;
        }

        // Blank line after title
        if row < inner.height {
            row += 1;
        }

        // Render theme options
        for (idx, &setting) in ThemeSetting::ALL.iter().enumerate() {
            if row >= inner.height.saturating_sub(2) {
                break;
            }
            let is_selected = idx == self.picker.selected;
            let is_current = setting == self.picker.current_setting;

            let pointer = if is_selected { "\u{276f} " } else { "  " };

            let (name_style, preview_style) = if is_selected {
                (
                    Style::default()
                        .fg(ratatui::style::Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                    Style::default().fg(ratatui::style::Color::White),
                )
            } else if is_current {
                (
                    Style::default()
                        .fg(ratatui::style::Color::Green)
                        .add_modifier(Modifier::BOLD),
                    Style::default().fg(ratatui::style::Color::DarkGray),
                )
            } else {
                (
                    Style::default(),
                    Style::default().fg(ratatui::style::Color::DarkGray),
                )
            };

            let current_marker = if is_current { " \u{2190}" } else { "" };

            // Get preview colors for this theme
            let (claude_c, perm_c, success_c, error_c, _border_c) = preview_colors(setting);

            // Build a compact color preview: colored squares showing key theme colors
            let label = format!("{:<36}", setting.label());
            let mut spans = vec![
                Span::styled(pointer, name_style),
                Span::styled(label, name_style),
            ];

            // Sample colored text: "Aa" in each key color
            spans.push(Span::styled("Aa", Style::default().fg(claude_c)));
            spans.push(Span::styled(" ", preview_style));
            spans.push(Span::styled("Aa", Style::default().fg(perm_c)));
            spans.push(Span::styled(" ", preview_style));
            spans.push(Span::styled("Aa", Style::default().fg(success_c)));
            spans.push(Span::styled(" ", preview_style));
            spans.push(Span::styled("Aa", Style::default().fg(error_c)));
            spans.push(Span::styled(current_marker, name_style));

            let line = Line::from(spans);
            buf.set_line(inner.x, inner.y + row, &line, inner.width);
            row += 1;
        }

        // Blank line
        if row < inner.height.saturating_sub(1) {
            row += 1;
        }

        // Hint line
        if row < inner.height {
            let hint = Line::from(Span::styled(
                "  \u{2191}\u{2193} select   Enter confirm   Esc cancel",
                Style::default().fg(ratatui::style::Color::DarkGray),
            ));
            buf.set_line(inner.x, inner.y + row, &hint, inner.width);
        }
    }
}
