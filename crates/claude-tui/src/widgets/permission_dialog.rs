use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Widget};

/// Word-wrap `text` to lines of at most `width` characters. Splits on
/// ASCII whitespace; words longer than `width` are kept whole and
/// allowed to overflow rather than mid-word-truncated.
fn wrap_words(text: &str, width: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if current.is_empty() {
            current.push_str(word);
        } else if current.len() + 1 + word.len() <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            out.push(std::mem::take(&mut current));
            current.push_str(word);
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

pub struct PermissionDialog {
    pub tool_name: String,
    pub description: String,
    pub input_preview: String,
    pub selected_button: usize, // 0=Allow, 1=Deny, 2=Always
    /// Optional Haiku-generated explanation of the command. Populated
    /// asynchronously after construction by spawning
    /// `populate_explanation_async`. Surfaces below the input preview
    /// when present.
    pub explanation: Option<String>,
}

impl PermissionDialog {
    pub fn new(tool_name: String, description: String, input_preview: String) -> Self {
        Self {
            tool_name,
            description,
            input_preview,
            selected_button: 0,
            explanation: None,
        }
    }

    /// Run the Haiku-backed permission explainer and return the
    /// generated explanation (or `None` when no secondary model is
    /// registered or the call errors). The TUI awaits this on a
    /// background task and stores the result via `set_explanation`.
    pub async fn fetch_explanation(
        tool_name: &str,
        tool_description: &str,
        input_preview: &str,
        conversation_context: &str,
    ) -> Option<String> {
        use claude_core::permission_explainer_prompt::explain_command;
        use tokio_util::sync::CancellationToken;
        explain_command(
            tool_name,
            tool_description,
            input_preview,
            conversation_context,
            CancellationToken::new(),
        )
        .await
        .ok()
        .flatten()
    }

    pub fn set_explanation(&mut self, text: Option<String>) {
        self.explanation = text;
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

        // Optional Haiku-generated explanation. When present and the
        // dialog has room, render in cyan beneath the preview. Wraps
        // word-by-word at the inner width to avoid horizontal overflow.
        if let Some(ref text) = self.explanation {
            let wrap_width = (inner.width.saturating_sub(2)) as usize;
            let wrapped = wrap_words(text, wrap_width.max(1));
            let max_lines = inner.height.saturating_sub(5) as usize;
            for (i, line) in wrapped.iter().take(max_lines).enumerate() {
                let row = inner.y + 4 + i as u16;
                if row >= inner.y + inner.height.saturating_sub(1) {
                    break;
                }
                let line_obj =
                    Line::from(Span::styled(line.clone(), Style::default().fg(Color::Cyan)));
                buf.set_line(inner.x + 1, row, &line_obj, inner.width.saturating_sub(2));
            }
        }

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
