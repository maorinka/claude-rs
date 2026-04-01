use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

pub struct TaskListEntry {
    pub id: String,
    pub subject: String,
    pub status: String,
}

pub struct TaskListWidget {
    pub tasks: Vec<TaskListEntry>,
    pub visible: bool,
}

impl TaskListWidget {
    pub fn new() -> Self { Self { tasks: Vec::new(), visible: false } }
    pub fn toggle(&mut self) { self.visible = !self.visible; }
}

impl Widget for &TaskListWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if !self.visible || area.height == 0 { return; }
        for (i, task) in self.tasks.iter().enumerate() {
            if i as u16 >= area.height { break; }
            let color = match task.status.as_str() {
                "completed" => Color::Green,
                "in_progress" => Color::Yellow,
                _ => Color::White,
            };
            let line = Line::from(vec![
                Span::styled(format!("[{}] ", task.status), Style::default().fg(color)),
                Span::raw(&task.subject),
            ]);
            buf.set_line(area.x, area.y + i as u16, &line, area.width);
        }
    }
}
