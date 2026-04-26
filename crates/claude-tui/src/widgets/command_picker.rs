use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Widget};

/// An entry shown in the command picker dropdown.
#[derive(Debug, Clone)]
pub struct CommandPickerEntry {
    pub name: String,
    pub description: String,
    pub display_name: Option<String>,
}

/// State for the slash-command picker overlay.
pub struct CommandPicker {
    /// All available commands.
    entries: Vec<CommandPickerEntry>,
    /// Filtered entries based on current query.
    filtered: Vec<usize>,
    /// Current filter query (text after the `/`).
    query: String,
    /// Index into `filtered` of the highlighted entry.
    selected: usize,
    /// Whether the picker is currently visible.
    pub visible: bool,
}

impl CommandPicker {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            filtered: Vec::new(),
            query: String::new(),
            selected: 0,
            visible: false,
        }
    }

    /// Populate the picker with available commands and show it.
    pub fn open(&mut self, entries: Vec<CommandPickerEntry>) {
        self.entries = entries;
        self.query.clear();
        self.selected = 0;
        self.visible = true;
        self.refilter();
    }

    /// Close the picker.
    pub fn close(&mut self) {
        self.visible = false;
        self.query.clear();
        self.filtered.clear();
    }

    /// Update the filter query and recompute matches.
    pub fn set_query(&mut self, query: &str) {
        self.query = query.to_string();
        self.selected = 0;
        self.refilter();
    }

    /// Move selection down.
    pub fn next(&mut self) {
        if !self.filtered.is_empty() {
            self.selected = (self.selected + 1) % self.filtered.len();
        }
    }

    /// Move selection up.
    pub fn prev(&mut self) {
        if !self.filtered.is_empty() {
            self.selected = self
                .selected
                .checked_sub(1)
                .unwrap_or(self.filtered.len() - 1);
        }
    }

    /// Return the selected command name, if any.
    pub fn selected_name(&self) -> Option<&str> {
        self.filtered
            .get(self.selected)
            .map(|&idx| self.entries[idx].name.as_str())
    }

    /// Whether the picker has any visible entries.
    pub fn has_entries(&self) -> bool {
        !self.filtered.is_empty()
    }

    /// Number of filtered entries (for dynamic sizing).
    pub fn filtered_count(&self) -> usize {
        self.filtered.len()
    }

    fn refilter(&mut self) {
        let q = self.query.to_lowercase();
        self.filtered = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                if q.is_empty() {
                    true
                } else {
                    // Match by command name starting with query (like TS).
                    e.name.to_lowercase().starts_with(&q)
                }
            })
            .map(|(i, _)| i)
            .collect();

        // If no prefix matches, fall back to substring match on name,
        // then description. This keeps discovery useful for terms like
        // "session", "cost", or "mcp" even when the command name differs.
        if self.filtered.is_empty() && !q.is_empty() {
            self.filtered = self
                .entries
                .iter()
                .enumerate()
                .filter(|(_, e)| {
                    e.name.to_lowercase().contains(&q) || e.description.to_lowercase().contains(&q)
                })
                .map(|(i, _)| i)
                .collect();
        }

        // Keep selected in bounds
        if self.selected >= self.filtered.len() {
            self.selected = 0;
        }
    }
}

impl Default for CommandPicker {
    fn default() -> Self {
        Self::new()
    }
}

/// Stateless widget that renders a `CommandPicker`.
pub struct CommandPickerWidget<'a> {
    picker: &'a CommandPicker,
    title: &'a str,
    prefix: &'a str,
}

impl<'a> CommandPickerWidget<'a> {
    pub fn new(picker: &'a CommandPicker) -> Self {
        Self {
            picker,
            title: "Commands",
            prefix: "/",
        }
    }

    pub fn titled(mut self, title: &'a str, prefix: &'a str) -> Self {
        self.title = title;
        self.prefix = prefix;
        self
    }
}

impl<'a> Widget for CommandPickerWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if !self.picker.visible || area.height < 3 {
            return;
        }

        // Clear the area first
        Clear.render(area, buf);

        let block = Block::default()
            .title(format!(" {} ", self.title))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));
        let inner = block.inner(area);
        block.render(area, buf);

        let max_visible = inner.height as usize;

        if self.picker.filtered.is_empty() {
            let line = Line::from(Span::styled(
                "No matching commands",
                Style::default().fg(Color::DarkGray),
            ));
            buf.set_line(inner.x, inner.y, &line, inner.width);
            return;
        }

        // Scroll so the selected item is always visible
        let scroll_offset = if self.picker.selected >= max_visible {
            self.picker.selected - max_visible + 1
        } else {
            0
        };

        let visible_entries = self
            .picker
            .filtered
            .iter()
            .skip(scroll_offset)
            .take(max_visible);

        for (row, &entry_idx) in visible_entries.enumerate() {
            let entry = &self.picker.entries[entry_idx];
            let display_name = entry.display_name.as_deref().unwrap_or(&entry.name);
            let is_selected = (row + scroll_offset) == self.picker.selected;

            let style = if is_selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            let line = Line::from(vec![
                Span::styled(
                    format!("{}{}", self.prefix, display_name),
                    style.add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  {}", entry.description),
                    if is_selected {
                        style
                    } else {
                        Style::default().fg(Color::DarkGray)
                    },
                ),
            ]);

            buf.set_line(inner.x, inner.y + row as u16, &line, inner.width);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;

    fn picker_with_entries() -> CommandPicker {
        let mut picker = CommandPicker::new();
        picker.open(vec![
            CommandPickerEntry {
                name: "doctor".into(),
                description: "Run environment health checks".into(),
                display_name: None,
            },
            CommandPickerEntry {
                name: "export".into(),
                description: "Export session to file".into(),
                display_name: None,
            },
        ]);
        picker
    }

    #[test]
    fn filters_by_description_when_name_does_not_match() {
        let mut picker = picker_with_entries();
        picker.set_query("session");

        assert_eq!(picker.filtered_count(), 1);
        assert_eq!(picker.selected_name(), Some("export"));
    }

    #[test]
    fn renders_empty_state_for_no_matches() {
        let mut picker = picker_with_entries();
        picker.set_query("zzzz");

        let area = Rect::new(0, 0, 40, 5);
        let mut buf = Buffer::empty(area);
        CommandPickerWidget::new(&picker).render(area, &mut buf);

        let rendered = buf
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("No matching commands"));
    }
}
