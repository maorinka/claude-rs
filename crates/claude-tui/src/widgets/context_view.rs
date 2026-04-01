use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

pub struct ContextView {
    pub system_tokens: u64,
    pub conversation_tokens: u64,
    pub tool_tokens: u64,
    pub total_tokens: u64,
    pub context_window: u64,
}

impl ContextView {
    pub fn new(total: u64, window: u64) -> Self {
        Self { system_tokens: 0, conversation_tokens: 0, tool_tokens: 0, total_tokens: total, context_window: window }
    }
}

impl Widget for &ContextView {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 { return; }
        let pct = if self.context_window > 0 { (self.total_tokens as f64 / self.context_window as f64 * 100.0) as u64 } else { 0 };
        let line = Line::from(vec![
            Span::raw(format!("Context: {} / {} tokens ({}%)", self.total_tokens, self.context_window, pct)),
        ]);
        buf.set_line(area.x, area.y, &line, area.width);
    }
}
