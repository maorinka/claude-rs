use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Widget};

/// Effort levels matching TS EffortLevel type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffortLevel {
    Low,
    Medium,
    High,
    Max,
}

impl EffortLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Max => "max",
        }
    }

    pub fn symbol(&self) -> &'static str {
        match self {
            Self::Low => "○",
            Self::Medium => "◐",
            Self::High => "●",
            Self::Max => "◉",
        }
    }
}

/// A model option in the picker.
#[derive(Debug, Clone)]
pub struct ModelOption {
    pub alias: String,
    pub display_name: String,
    pub description: String,
    pub model_id: String,
    pub supports_effort: bool,
    pub supports_max_effort: bool,
}

/// Interactive model picker rendered inline below the prompt.
/// ↑↓ to navigate models, ← → to adjust effort, Enter to confirm, Esc to cancel.
/// Matches the TS ModelPicker component behavior.
pub struct ModelPicker {
    options: Vec<ModelOption>,
    pub selected: usize,
    pub visible: bool,
    pub effort: EffortLevel,
    pub current_model: String,
}

impl ModelPicker {
    pub fn new() -> Self {
        Self {
            options: default_model_options(),
            selected: 0,
            visible: false,
            effort: EffortLevel::High, // Default effort
            current_model: String::new(),
        }
    }

    /// Open the picker, highlighting the current model.
    pub fn open(&mut self, current_model: &str) {
        self.current_model = current_model.to_string();
        self.visible = true;
        self.effort = EffortLevel::High;

        // Find and highlight current model
        self.selected = self
            .options
            .iter()
            .position(|o| o.model_id == current_model || o.alias == current_model)
            .unwrap_or(0);
    }

    pub fn close(&mut self) {
        self.visible = false;
    }

    /// Move selection down.
    pub fn next(&mut self) {
        if !self.options.is_empty() {
            self.selected = (self.selected + 1) % self.options.len();
            // Reset effort to default for new model
            self.effort = self.default_effort_for_selected();
        }
    }

    /// Move selection up.
    pub fn prev(&mut self) {
        if !self.options.is_empty() {
            self.selected = self
                .selected
                .checked_sub(1)
                .unwrap_or(self.options.len() - 1);
            self.effort = self.default_effort_for_selected();
        }
    }

    /// Cycle effort right (low → medium → high → max → low).
    pub fn effort_right(&mut self) {
        if let Some(opt) = self.options.get(self.selected) {
            if !opt.supports_effort {
                return;
            }
            let include_max = opt.supports_max_effort;
            self.effort = match self.effort {
                EffortLevel::Low => EffortLevel::Medium,
                EffortLevel::Medium => EffortLevel::High,
                EffortLevel::High => {
                    if include_max {
                        EffortLevel::Max
                    } else {
                        EffortLevel::Low
                    }
                }
                EffortLevel::Max => EffortLevel::Low,
            };
        }
    }

    /// Cycle effort left (reverse).
    pub fn effort_left(&mut self) {
        if let Some(opt) = self.options.get(self.selected) {
            if !opt.supports_effort {
                return;
            }
            let include_max = opt.supports_max_effort;
            self.effort = match self.effort {
                EffortLevel::Low => {
                    if include_max {
                        EffortLevel::Max
                    } else {
                        EffortLevel::High
                    }
                }
                EffortLevel::Medium => EffortLevel::Low,
                EffortLevel::High => EffortLevel::Medium,
                EffortLevel::Max => EffortLevel::High,
            };
        }
    }

    /// Confirm selection. Returns (model_id, effort_if_supported).
    pub fn confirm(&mut self) -> Option<(String, Option<EffortLevel>)> {
        let opt = self.options.get(self.selected)?;
        let model = opt.model_id.clone();
        let effort = if opt.supports_effort {
            Some(self.effort)
        } else {
            None
        };
        self.close();
        Some((model, effort))
    }

    pub fn selected_option(&self) -> Option<&ModelOption> {
        self.options.get(self.selected)
    }

    pub fn options(&self) -> &[ModelOption] {
        &self.options
    }

    /// Height needed to render (for layout).
    pub fn height(&self) -> u16 {
        // models + effort line + border (2) + hint
        (self.options.len() as u16) + 4
    }

    fn default_effort_for_selected(&self) -> EffortLevel {
        EffortLevel::High
    }
}

impl Default for ModelPicker {
    fn default() -> Self {
        Self::new()
    }
}

/// Build model options matching TS getModelOptions() order and content.
/// Order: Default → Sonnet → Sonnet 1M → Opus → Opus 1M → Haiku
/// (Pro/Standard/Enterprise order — Sonnet default)
/// For Max/Team Premium: Default → Opus 1M → Sonnet → Sonnet 1M → Haiku
fn default_model_options() -> Vec<ModelOption> {
    // Default order for most users (Pro/Enterprise/PAYG)
    // TS: getModelOptionsBase → standardOptions path
    vec![
        ModelOption {
            alias: "sonnet".into(),
            display_name: "Sonnet".into(),
            description: "Sonnet 4.6 · Best for everyday tasks · $3/$15/Mtok".into(),
            model_id: "claude-sonnet-4-6".into(),
            supports_effort: true,
            supports_max_effort: false,
        },
        ModelOption {
            alias: "sonnet[1m]".into(),
            display_name: "Sonnet (1M context)".into(),
            description: "Sonnet 4.6 for long sessions · $3/$15/Mtok".into(),
            model_id: "claude-sonnet-4-6[1m]".into(),
            supports_effort: true,
            supports_max_effort: false,
        },
        ModelOption {
            alias: "opus".into(),
            display_name: "Opus".into(),
            description: "Opus 4.6 · Most capable for complex work · $15/$75/Mtok".into(),
            model_id: "claude-opus-4-6".into(),
            supports_effort: true,
            supports_max_effort: true,
        },
        ModelOption {
            alias: "opus[1m]".into(),
            display_name: "Opus (1M context)".into(),
            description: "Opus 4.6 with 1M context · Most capable · $15/$75/Mtok".into(),
            model_id: "claude-opus-4-6[1m]".into(),
            supports_effort: true,
            supports_max_effort: true,
        },
        ModelOption {
            alias: "haiku".into(),
            display_name: "Haiku".into(),
            description: "Haiku 4.5 · Fastest for quick answers · $0.80/$4/Mtok".into(),
            model_id: "claude-haiku-4-5-20251001".into(),
            supports_effort: false,
            supports_max_effort: false,
        },
    ]
}

/// Stateless widget that renders the model picker inline.
pub struct ModelPickerWidget<'a> {
    picker: &'a ModelPicker,
}

impl<'a> ModelPickerWidget<'a> {
    pub fn new(picker: &'a ModelPicker) -> Self {
        Self { picker }
    }
}

impl<'a> Widget for ModelPickerWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if !self.picker.visible || area.height < 4 {
            return;
        }

        Clear.render(area, buf);

        let block = Block::default()
            .title(" Select Model ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));
        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height == 0 || inner.width == 0 {
            return;
        }

        let mut row: u16 = 0;

        // Render model list
        for (idx, option) in self.picker.options.iter().enumerate() {
            if row >= inner.height.saturating_sub(2) {
                break;
            }
            let is_selected = idx == self.picker.selected;
            let is_current = option.model_id == self.picker.current_model;

            let pointer = if is_selected { "❯ " } else { "  " };

            let (name_style, desc_style) = if is_selected {
                (
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                    Style::default().fg(Color::White),
                )
            } else if is_current {
                (
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                    Style::default().fg(Color::DarkGray),
                )
            } else {
                (Style::default(), Style::default().fg(Color::DarkGray))
            };

            let current_marker = if is_current { " ←" } else { "" };

            let line = Line::from(vec![
                Span::styled(pointer, name_style),
                Span::styled(format!("{:<18}", option.display_name), name_style),
                Span::styled(
                    format!("{}{}", option.description, current_marker),
                    desc_style,
                ),
            ]);

            buf.set_line(inner.x, inner.y + row, &line, inner.width);
            row += 1;
        }

        // Effort indicator line (below model list)
        if row < inner.height.saturating_sub(1) {
            row += 1; // blank line
        }
        if row < inner.height {
            let line = if let Some(opt) = self.picker.selected_option() {
                if opt.supports_effort {
                    let effort = self.picker.effort;
                    // Show all effort levels, highlight current
                    let mut spans = vec![Span::styled("  ", Style::default())];
                    for level in &[
                        EffortLevel::Low,
                        EffortLevel::Medium,
                        EffortLevel::High,
                        EffortLevel::Max,
                    ] {
                        if *level == EffortLevel::Max && !opt.supports_max_effort {
                            continue;
                        }
                        let is_active = *level == effort;
                        let style = if is_active {
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::DarkGray)
                        };
                        spans.push(Span::styled(
                            format!("{} {} ", level.symbol(), level.as_str()),
                            style,
                        ));
                    }
                    spans.push(Span::styled(
                        "  ← → to adjust",
                        Style::default().fg(Color::DarkGray),
                    ));
                    Line::from(spans)
                } else {
                    Line::from(Span::styled(
                        "  ○ Effort not supported for this model",
                        Style::default().fg(Color::DarkGray),
                    ))
                }
            } else {
                Line::default()
            };
            buf.set_line(inner.x, inner.y + row, &line, inner.width);
            row += 1;
        }

        // Hint line
        if row < inner.height {
            let hint = Line::from(Span::styled(
                "  ↑↓ select   ← → effort   Enter confirm   Esc cancel",
                Style::default().fg(Color::DarkGray),
            ));
            buf.set_line(inner.x, inner.y + row, &hint, inner.width);
        }
    }
}
