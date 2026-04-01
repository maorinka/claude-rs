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
                    e.name.to_lowercase().contains(&q)
                        || e.description.to_lowercase().contains(&q)
                }
            })
            .map(|(i, _)| i)
            .collect();
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
}

impl<'a> CommandPickerWidget<'a> {
    pub fn new(picker: &'a CommandPicker) -> Self {
        Self { picker }
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
            .title(" Commands ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));
        let inner = block.inner(area);
        block.render(area, buf);

        let max_visible = inner.height as usize;

        // Scroll so the selected item is always visible
        let scroll_offset = if self.picker.selected >= max_visible {
            self.picker.selected - max_visible + 1
        } else {
            0
        };

        let visible_entries = self.picker.filtered.iter()
            .skip(scroll_offset)
            .take(max_visible);

        for (row, &entry_idx) in visible_entries.enumerate() {
            let entry = &self.picker.entries[entry_idx];
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
                    format!("/{}", entry.name),
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
