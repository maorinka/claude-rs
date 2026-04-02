use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Widget};

pub struct PermissionDialog {
    pub tool_name: String,
    pub description: String,
    pub input_preview: String,
    pub selected_button: usize, // 0=Allow, 1=Deny, 2=Always
}

impl PermissionDialog {
    pub fn new(tool_name: String, description: String, input_preview: String) -> Self {
        Self {
            tool_name,
            description,
            input_preview,
            selected_button: 0,
        }
    }

    pub fn next_button(&mut self) {
        self.selected_button = (self.selected_button + 1) % 3;
    }
    pub fn prev_button(&mut self) {
        self.selected_button = (self.selected_button + 2) % 3;
    }
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

        if inner.height < 4 {
            return;
        }

        // Description
        let desc = Line::from(Span::raw(self.description.clone()));
        buf.set_line(inner.x + 1, inner.y, &desc, inner.width.saturating_sub(2));

        // Input preview (truncated)
        let preview = if self.input_preview.len() > inner.width as usize - 4 {
            format!("{}...", &self.input_preview[..inner.width as usize - 7])
        } else {
            self.input_preview.clone()
        };
        let preview_line = Line::from(Span::styled(preview, Style::default().fg(Color::DarkGray)));
        buf.set_line(
            inner.x + 1,
            inner.y + 2,
            &preview_line,
            inner.width.saturating_sub(2),
        );

        // Buttons at bottom
        let button_y = inner.y + inner.height - 1;
        let buttons = ["Allow", "Deny", "Always Allow"];
        let mut x = inner.x + 2;
        for (i, label) in buttons.iter().enumerate() {
            let style = if i == self.selected_button {
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
