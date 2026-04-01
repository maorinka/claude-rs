use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Widget};

/// Metadata about what was compacted.
#[derive(Clone, Debug)]
pub struct CompactionSummary {
    /// Number of messages that were removed / summarized.
    pub messages_removed: usize,
    /// Number of tokens freed by compaction.
    pub tokens_freed: u64,
    /// Token length of the new summary that replaced the removed messages.
    pub summary_tokens: u64,
    /// Total tokens before compaction.
    pub tokens_before: u64,
    /// Total tokens after compaction.
    pub tokens_after: u64,
    /// Optional user-provided context that guided the compaction.
    pub user_context: Option<String>,
    /// Whether this was an automatic compaction (vs. manual /compact command).
    pub is_auto: bool,
    /// Direction of compaction.
    pub direction: CompactionDirection,
}

/// Which direction the compaction operated.
#[derive(Clone, Debug, PartialEq)]
pub enum CompactionDirection {
    /// Summarized messages up to the compaction point.
    UpTo,
    /// Summarized messages from the compaction point onward.
    FromPoint,
}

impl CompactionSummary {
    /// Net tokens saved (freed minus summary overhead).
    pub fn net_tokens_saved(&self) -> i64 {
        self.tokens_freed as i64 - self.summary_tokens as i64
    }

    /// Percentage of context freed.
    pub fn freed_percentage(&self) -> f64 {
        if self.tokens_before == 0 {
            return 0.0;
        }
        self.tokens_freed as f64 / self.tokens_before as f64 * 100.0
    }

    /// Produce a plain-text report.
    pub fn to_text_report(&self) -> String {
        let mut lines = Vec::new();
        lines.push("=== Compaction Summary ===".into());
        lines.push(String::new());

        let kind = if self.is_auto { "Auto-compaction" } else { "Manual compaction" };
        let direction = match self.direction {
            CompactionDirection::UpTo => "up to this point",
            CompactionDirection::FromPoint => "from this point",
        };
        lines.push(format!("{} ({})", kind, direction));
        lines.push(String::new());

        lines.push(format!(
            "Messages summarized: {}",
            self.messages_removed
        ));
        lines.push(format!(
            "Tokens freed:        {} ({:.1}% of context)",
            format_tokens(self.tokens_freed),
            self.freed_percentage(),
        ));
        lines.push(format!(
            "Summary length:      {} tokens",
            format_tokens(self.summary_tokens),
        ));
        lines.push(format!(
            "Net tokens saved:    {}",
            format_tokens_signed(self.net_tokens_saved()),
        ));
        lines.push(format!(
            "Context before:      {}",
            format_tokens(self.tokens_before),
        ));
        lines.push(format!(
            "Context after:       {}",
            format_tokens(self.tokens_after),
        ));

        if let Some(ref ctx) = self.user_context {
            lines.push(String::new());
            lines.push(format!("Guided by: \"{}\"", ctx));
        }

        lines.join("\n")
    }
}

/// Format a token count with `k`/`M` suffix.
fn format_tokens(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.1}k", tokens as f64 / 1_000.0)
    } else {
        format!("{}", tokens)
    }
}

/// Format a signed token count.
fn format_tokens_signed(tokens: i64) -> String {
    let abs = tokens.unsigned_abs();
    let formatted = format_tokens(abs);
    if tokens >= 0 {
        format!("+{}", formatted)
    } else {
        format!("-{}", formatted)
    }
}

/// Widget that renders the compaction summary in the TUI.
pub struct CompactSummaryWidget<'a> {
    pub summary: &'a CompactionSummary,
}

impl<'a> CompactSummaryWidget<'a> {
    pub fn new(summary: &'a CompactionSummary) -> Self {
        Self { summary }
    }
}

impl Widget for CompactSummaryWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        Clear.render(area, buf);

        let title = if self.summary.is_auto {
            " Auto-compacted "
        } else {
            " Compaction Summary "
        };

        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Blue));

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height < 3 {
            return;
        }

        let mut row = inner.y;

        // Header: "Summarized N messages"
        let direction_text = match self.summary.direction {
            CompactionDirection::UpTo => "up to this point",
            CompactionDirection::FromPoint => "from this point",
        };
        let header = Line::from(vec![
            Span::styled(
                "Summarized ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{}", self.summary.messages_removed),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" messages {}", direction_text),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]);
        buf.set_line(inner.x + 1, row, &header, inner.width.saturating_sub(2));
        row += 1;

        // Token savings
        if row < inner.y + inner.height {
            let net = self.summary.net_tokens_saved();
            let net_color = if net > 0 { Color::Green } else { Color::Red };
            let savings = Line::from(vec![
                Span::styled(
                    format!(
                        "Freed {} tokens ({:.1}%), ",
                        format_tokens(self.summary.tokens_freed),
                        self.summary.freed_percentage(),
                    ),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    format!("net {}", format_tokens_signed(net)),
                    Style::default().fg(net_color),
                ),
            ]);
            buf.set_line(inner.x + 1, row, &savings, inner.width.saturating_sub(2));
            row += 1;
        }

        // Context before/after
        if row < inner.y + inner.height {
            let context_line = Line::from(Span::styled(
                format!(
                    "Context: {} -> {}",
                    format_tokens(self.summary.tokens_before),
                    format_tokens(self.summary.tokens_after),
                ),
                Style::default().fg(Color::DarkGray),
            ));
            buf.set_line(inner.x + 1, row, &context_line, inner.width.saturating_sub(2));
            row += 1;
        }

        // User context if present
        if let Some(ref ctx) = self.summary.user_context {
            if row < inner.y + inner.height {
                row += 1; // blank line
                if row < inner.y + inner.height {
                    let ctx_line = Line::from(Span::styled(
                        format!("Context: \"{}\"", ctx),
                        Style::default()
                            .fg(Color::DarkGray)
                            .add_modifier(Modifier::ITALIC),
                    ));
                    buf.set_line(inner.x + 1, row, &ctx_line, inner.width.saturating_sub(2));
                }
            }
        }

        // Hint at bottom
        let hint_y = inner.y + inner.height - 1;
        if hint_y > row {
            let hint = Line::from(Span::styled(
                "ctrl+o to view full history",
                Style::default().fg(Color::DarkGray),
            ));
            buf.set_line(inner.x + 1, hint_y, &hint, inner.width.saturating_sub(2));
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_summary() -> CompactionSummary {
        CompactionSummary {
            messages_removed: 42,
            tokens_freed: 15_000,
            summary_tokens: 2_000,
            tokens_before: 180_000,
            tokens_after: 167_000,
            user_context: None,
            is_auto: false,
            direction: CompactionDirection::UpTo,
        }
    }

    #[test]
    fn test_net_tokens_saved() {
        let s = sample_summary();
        assert_eq!(s.net_tokens_saved(), 13_000); // 15000 - 2000
    }

    #[test]
    fn test_freed_percentage() {
        let s = sample_summary();
        let pct = s.freed_percentage();
        // 15000 / 180000 * 100 = 8.333...
        assert!((pct - 8.333).abs() < 0.01, "Expected ~8.33%, got {}", pct);
    }

    #[test]
    fn test_freed_percentage_zero_before() {
        let mut s = sample_summary();
        s.tokens_before = 0;
        assert_eq!(s.freed_percentage(), 0.0);
    }

    #[test]
    fn test_text_report_contains_key_info() {
        let s = sample_summary();
        let report = s.to_text_report();
        assert!(report.contains("42"), "Should show messages removed");
        assert!(report.contains("15.0k"), "Should show tokens freed");
        assert!(report.contains("+13.0k"), "Should show net savings");
        assert!(report.contains("Manual"), "Should show compaction type");
        assert!(report.contains("up to this point"), "Should show direction");
    }

    #[test]
    fn test_text_report_with_user_context() {
        let mut s = sample_summary();
        s.user_context = Some("focus on the auth module".into());
        let report = s.to_text_report();
        assert!(
            report.contains("focus on the auth module"),
            "Should include user context"
        );
    }

    #[test]
    fn test_text_report_auto_compaction() {
        let mut s = sample_summary();
        s.is_auto = true;
        let report = s.to_text_report();
        assert!(report.contains("Auto-compaction"), "Should indicate auto");
    }

    #[test]
    fn test_format_tokens() {
        assert_eq!(format_tokens(500), "500");
        assert_eq!(format_tokens(1_500), "1.5k");
        assert_eq!(format_tokens(200_000), "200.0k");
        assert_eq!(format_tokens(1_500_000), "1.5M");
    }

    #[test]
    fn test_format_tokens_signed() {
        assert_eq!(format_tokens_signed(13_000), "+13.0k");
        assert_eq!(format_tokens_signed(-5_000), "-5.0k");
        assert_eq!(format_tokens_signed(0), "+0");
    }

    #[test]
    fn test_widget_renders_without_panic() {
        let s = sample_summary();
        let widget = CompactSummaryWidget::new(&s);
        let area = Rect::new(0, 0, 60, 10);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);
        let content: String = buf
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(
            content.contains("Summarized") || content.contains("Compaction"),
            "Buffer should contain compaction content"
        );
    }
}
