use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Widget};
use serde_json::Value;

/// Word-wrap `text` to lines of at most `width` characters. Splits on
/// ASCII whitespace; words longer than `width` are kept whole and
/// allowed to overflow rather than mid-word-truncated.
fn wrap_words(text: &str, width: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if current.is_empty() {
            current.push_str(word);
        } else if current.chars().count() + 1 + word.chars().count() <= width {
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
    pub selected_button: usize, // 0=Yes, 1=Yes always, 2=No
    /// Optional Haiku-generated explanation of the command. Populated
    /// asynchronously after construction by spawning
    /// `populate_explanation_async`. Surfaces below the input preview
    /// when present.
    pub explanation: Option<String>,
    pub explanation_visible: bool,
}

impl PermissionDialog {
    pub fn new(tool_name: String, description: String, input_preview: String) -> Self {
        Self {
            tool_name,
            description,
            input_preview,
            selected_button: 0,
            explanation: None,
            explanation_visible: false,
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

    pub fn toggle_explanation(&mut self) {
        self.explanation_visible = !self.explanation_visible;
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
            1 => "always",
            2 => "deny",
            _ => "allow",
        }
    }

    pub fn height(&self) -> u16 {
        let mut height = 10;
        if self.explanation_visible {
            height += self
                .explanation
                .as_deref()
                .map_or(1, |text| wrap_words(text, 72).len().clamp(1, 4) as u16);
        }
        height
    }
}

impl Widget for &PermissionDialog {
    fn render(self, area: Rect, buf: &mut Buffer) {
        Clear.render(area, buf);

        if area.height < 4 || area.width == 0 {
            return;
        }

        let permission = Color::Rgb(177, 185, 249);
        let inactive = Color::Rgb(153, 153, 153);
        let mut y = area.y;
        let title = format!(" {}", permission_title(&self.tool_name));
        let title_width = title.chars().count();
        let dashes = area.width as usize;
        let top = Line::from(vec![
            Span::styled(
                "\u{2500}".repeat(dashes.min(2)),
                Style::default().fg(permission),
            ),
            Span::styled(title, Style::default().fg(Color::Reset)),
            Span::styled(
                "\u{2500}".repeat(dashes.saturating_sub(title_width + 2)),
                Style::default().fg(permission),
            ),
        ]);
        buf.set_line(area.x, y, &top, area.width);
        y += 1;

        let (primary, secondary) = permission_preview(&self.tool_name, &self.input_preview);
        if y < area.y + area.height {
            let line = Line::from(Span::styled(
                format!(
                    "   {}",
                    truncate_with_ellipsis(&primary, area.width.saturating_sub(3) as usize)
                ),
                Style::default().fg(inactive),
            ));
            buf.set_line(area.x, y, &line, area.width);
            y += 1;
        }
        if let Some(secondary) = secondary {
            if y < area.y + area.height {
                let line = Line::from(Span::styled(
                    format!(
                        "   {}",
                        truncate_with_ellipsis(&secondary, area.width.saturating_sub(3) as usize)
                    ),
                    Style::default().fg(inactive),
                ));
                buf.set_line(area.x, y, &line, area.width);
                y += 1;
            }
        } else if !self.description.trim().is_empty() && y < area.y + area.height {
            let line = Line::from(Span::styled(
                format!(
                    "   {}",
                    truncate_with_ellipsis(
                        &self.description,
                        area.width.saturating_sub(3) as usize
                    )
                ),
                Style::default().fg(inactive),
            ));
            buf.set_line(area.x, y, &line, area.width);
            y += 1;
        }

        if self.explanation_visible {
            let text = self
                .explanation
                .as_deref()
                .unwrap_or("Generating explanation...");
            let wrap_width = area.width.saturating_sub(3) as usize;
            for line in wrap_words(text, wrap_width.max(1)).into_iter().take(4) {
                if y >= area.y + area.height {
                    break;
                }
                let line = Line::from(Span::styled(
                    format!("   {}", line),
                    Style::default().fg(Color::Cyan),
                ));
                buf.set_line(area.x, y, &line, area.width);
                y += 1;
            }
        }

        y += 1;
        if y < area.y + area.height {
            let question = Line::from(Span::raw(" Do you want to proceed?"));
            buf.set_line(area.x, y, &question, area.width);
            y += 1;
        }

        let always_label = format!(
            "Yes, and don't ask again for: {}",
            permission_rule_preview(&self.tool_name, &self.input_preview)
        );
        let options = ["Yes".to_string(), always_label, "No".to_string()];
        for (i, label) in options.iter().enumerate() {
            if y >= area.y + area.height {
                break;
            }
            let selector = if i == self.selected_button {
                "\u{276F}"
            } else {
                " "
            };
            let selector_style = if i == self.selected_button {
                Style::default().fg(permission).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(inactive)
            };
            let line = Line::from(vec![
                Span::styled(format!("  {selector} "), selector_style),
                Span::raw(format!(
                    "{}. {}",
                    i + 1,
                    truncate_with_ellipsis(label, area.width.saturating_sub(7) as usize)
                )),
            ]);
            buf.set_line(area.x, y, &line, area.width);
            y += 1;
        }

        y += 1;
        if y < area.y + area.height {
            let explain = if self.explanation_visible {
                "hide"
            } else {
                "explain"
            };
            let footer = Line::from(Span::styled(
                format!(" Esc to cancel \u{00b7} Tab to amend \u{00b7} ctrl+e to {explain}"),
                Style::default().fg(inactive),
            ));
            buf.set_line(area.x, y, &footer, area.width);
        }
    }
}

fn permission_title(tool_name: &str) -> String {
    if tool_name == "Bash" {
        "Bash command".to_string()
    } else {
        format!("{tool_name} permission")
    }
}

fn permission_preview(tool_name: &str, input_preview: &str) -> (String, Option<String>) {
    if tool_name == "Bash" {
        if let Ok(value) = serde_json::from_str::<Value>(input_preview) {
            let command = value
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or(input_preview)
                .to_string();
            let description = value
                .get("description")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string);
            return (command, description);
        }
    }
    (input_preview.to_string(), None)
}

fn permission_rule_preview(tool_name: &str, input_preview: &str) -> String {
    let (preview, _) = permission_preview(tool_name, input_preview);
    if tool_name == "Bash" {
        preview
    } else {
        tool_name.to_string()
    }
}

fn truncate_with_ellipsis(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    if max_chars <= 3 {
        return s.chars().take(max_chars).collect();
    }
    let prefix: String = s.chars().take(max_chars - 3).collect();
    format!("{prefix}...")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_unicode_preview_without_splitting_codepoints() {
        assert_eq!(truncate_with_ellipsis("éééabcdef", 6), "ééé...");
    }

    #[test]
    fn renders_on_tiny_width_without_panicking() {
        let dialog = PermissionDialog::new("Bash".into(), "Run command".into(), "ééééé".into());
        let area = Rect::new(0, 0, 8, 6);
        let mut buf = Buffer::empty(area);
        (&dialog).render(area, &mut buf);
    }

    #[test]
    fn selected_option_order_matches_ts_prompt() {
        let mut dialog = PermissionDialog::new("Bash".into(), "Run command".into(), "{}".into());
        assert_eq!(dialog.selected(), "allow");
        dialog.next_button();
        assert_eq!(dialog.selected(), "always");
        dialog.next_button();
        assert_eq!(dialog.selected(), "deny");
    }
}
