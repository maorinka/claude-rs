use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Widget};

pub struct PermissionDialog {
    pub tool_name: String,
    pub description: String,
    pub input_preview: String,
    pub selected_button: usize,  // 0=Allow, 1=Deny, 2=Always
}

impl PermissionDialog {
    pub fn new(tool_name: String, description: String, input_preview: String) -> Self {
        Self { tool_name, description, input_preview, selected_button: 0 }
    }

    pub fn next_button(&mut self) { self.selected_button = (self.selected_button + 1) % 3; }
    pub fn prev_button(&mut self) { self.selected_button = (self.selected_button + 2) % 3; }
    pub fn selected(&self) -> &str {
        match self.selected_button {
            0 => "allow",
            1 => "deny",
            2 => "always",
            _ => "allow",
        }
    }
}

impl Widget for &PermissionDialog {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Clear background
        Clear.render(area, buf);

        let block = Block::default()
            .title(format!(" {} ", self.tool_name))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow));

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height < 4 { return; }

        // Description
        let desc = Line::from(Span::raw(self.description.clone()));
        buf.set_line(inner.x + 1, inner.y, &desc, inner.width.saturating_sub(2));

        // Input preview (truncated)
        let max_preview = inner.width as usize - 4;
        let preview = if self.input_preview.len() > max_preview {
            let take_chars = inner.width as usize - 7;
            let truncated = &self.input_preview[..take_chars];
            format!("{}...", truncated)
        } else {
            self.input_preview.clone()
        };
        let preview_line = Line::from(Span::styled(preview, Style::default().fg(Color::DarkGray)));
        buf.set_line(inner.x + 1, inner.y + 2, &preview_line, inner.width.saturating_sub(2));

        // Buttons at bottom
        let button_y = inner.y + inner.height - 1;
        let buttons = ["Allow", "Deny", "Always Allow"];
        let mut x = inner.x + 2;
        for (i, label) in buttons.iter().enumerate() {
            let style = if i == self.selected_button {
                Style::default().fg(Color::Black).bg(Color::White).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            let span = Span::styled(format!(" {} ", label), style);
            buf.set_span(x, button_y, &span, span.width() as u16);
            x += span.width() as u16 + 2;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to render a PermissionDialog into a buffer of a given size.
    fn render_dialog(width: u16, height: u16, input_preview: &str) {
        let dialog = PermissionDialog::new(
            "Bash".into(),
            "Execute command".into(),
            input_preview.into(),
        );
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        (&dialog).render(area, &mut buf);
    }

    #[test]
    fn test_permission_dialog_width_zero() {
        render_dialog(0, 10, "ls -la");
    }

    #[test]
    fn test_permission_dialog_width_one() {
        render_dialog(1, 10, "ls -la");
    }

    #[test]
    fn test_permission_dialog_width_six() {
        render_dialog(6, 10, "ls -la");
    }

    #[test]
    fn test_permission_dialog_width_seven() {
        render_dialog(7, 10, "ls -la");
    }

    #[test]
    fn test_permission_dialog_width_eight() {
        render_dialog(8, 10, "ls -la");
    }

    #[test]
    fn test_permission_dialog_small_height() {
        render_dialog(40, 3, "ls -la");
    }

    #[test]
    fn test_permission_dialog_multibyte_input_preview() {
        let preview: String = std::iter::repeat('\u{1F600}').take(100).collect();
        render_dialog(20, 10, &preview);
    }

    #[test]
    fn test_permission_dialog_cjk_input_preview_narrow_width() {
        let preview: String = std::iter::repeat('\u{4E16}').take(50).collect();
        render_dialog(5, 10, &preview);
    }

    #[test]
    fn test_permission_dialog_multibyte_input_width_zero() {
        let preview: String = std::iter::repeat('\u{4E16}').take(50).collect();
        render_dialog(0, 10, &preview);
    }
}
