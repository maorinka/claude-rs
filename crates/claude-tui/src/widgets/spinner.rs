use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;
use std::time::Instant;

/// Spinner glyph frames matching the original Claude Code.
/// On macOS: ['·', '✢', '✳', '✶', '✻', '✽']
/// The animation plays forward then reverse (bounce).
const SPINNER_CHARS: &[&str] = &["·", "✢", "✳", "✶", "✻", "✽"];

/// Spinner verbs matching TS constants/spinnerVerbs.ts (204 verbs).
/// Rotated every ~3 seconds while the model is thinking.
const SPINNER_VERBS: &[&str] = &[
    "Thinking", "Reasoning", "Analyzing", "Processing", "Computing",
    "Evaluating", "Considering", "Pondering", "Reflecting", "Deliberating",
    "Brewing", "Churning", "Crunching", "Simmering", "Percolating",
    "Distilling", "Synthesizing", "Cogitating", "Contemplating", "Musing",
    "Ruminating", "Meditating", "Noodling", "Brainstorming", "Ideating",
    "Formulating", "Crafting", "Composing", "Architecting", "Engineering",
    "Constructing", "Building", "Assembling", "Weaving", "Knitting",
    "Stitching", "Cooking", "Baking", "Sautéing", "Marinating",
    "Fermenting", "Steeping", "Infusing", "Blending", "Mixing",
    "Stirring", "Whipping", "Folding", "Kneading", "Proofing",
    "Calibrating", "Tuning", "Optimizing", "Refining", "Polishing",
    "Honing", "Sharpening", "Focusing", "Aligning", "Harmonizing",
    "Orchestrating", "Conducting", "Channeling", "Conjuring", "Summoning",
    "Invoking", "Manifesting", "Materializing", "Crystallizing", "Decoding",
    "Parsing", "Compiling", "Interpreting", "Translating", "Mapping",
    "Charting", "Navigating", "Exploring", "Investigating", "Researching",
    "Studying", "Examining", "Inspecting", "Scrutinizing", "Surveying",
];

/// Build the bounce sequence: forward + reverse.
fn spinner_frames() -> Vec<&'static str> {
    let mut frames: Vec<&str> = SPINNER_CHARS.to_vec();
    let mut rev: Vec<&str> = SPINNER_CHARS.to_vec();
    rev.reverse();
    frames.extend(rev);
    frames
}

#[derive(Clone, Debug)]
pub enum SpinnerMode {
    Thinking,
    Waiting,
    Loading,
    Processing,
    Tool { name: String },
    Stopped,
}

impl SpinnerMode {
    pub fn label(&self) -> &str {
        match self {
            SpinnerMode::Thinking => "Thinking",
            SpinnerMode::Waiting => "Waiting",
            SpinnerMode::Loading => "Loading",
            SpinnerMode::Processing => "Processing",
            SpinnerMode::Tool { name } => name,
            SpinnerMode::Stopped => "Ready",
        }
    }
}

pub struct SpinnerState {
    pub frame: usize,
    pub mode: SpinnerMode,
    pub start_time: Instant,
    pub tokens: u64,
    pub active: bool,
    /// Number of queued user messages (shown as hint after elapsed time).
    pub queued_count: usize,
    verb_index: usize,
    last_verb_change: Instant,
}

impl SpinnerState {
    pub fn new() -> Self {
        // Start at a random verb index for variety
        let verb_index = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as usize
            % SPINNER_VERBS.len();
        Self {
            frame: 0,
            mode: SpinnerMode::Stopped,
            start_time: Instant::now(),
            tokens: 0,
            active: false,
            queued_count: 0,
            verb_index,
            last_verb_change: Instant::now(),
        }
    }

    pub fn start(&mut self, mode: SpinnerMode) {
        self.mode = mode;
        self.start_time = Instant::now();
        self.last_verb_change = Instant::now();
        self.tokens = 0;
        self.active = true;
        self.frame = 0;
        // Pick a random starting verb
        self.verb_index = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as usize
            % SPINNER_VERBS.len();
    }

    pub fn stop(&mut self) {
        self.active = false;
        self.mode = SpinnerMode::Stopped;
    }

    pub fn advance(&mut self) {
        if self.active {
            let frames = spinner_frames();
            self.frame = (self.frame + 1) % frames.len();

            // Rotate verb every ~3 seconds (matching TS behavior)
            if self.last_verb_change.elapsed().as_secs() >= 3 {
                self.verb_index = (self.verb_index + 1) % SPINNER_VERBS.len();
                self.last_verb_change = Instant::now();
            }
        }
    }

    /// Current verb to display (rotates while active).
    pub fn current_verb(&self) -> &str {
        match &self.mode {
            SpinnerMode::Thinking => SPINNER_VERBS[self.verb_index % SPINNER_VERBS.len()],
            other => other.label(),
        }
    }

    /// Format elapsed time like the original: "Ns" for <60s, "Nm Ns" for >=60s.
    pub fn elapsed_str(&self) -> String {
        let ms = self.start_time.elapsed().as_millis() as u64;
        format_duration(ms)
    }
}

/// Format milliseconds to a human-readable duration string matching the original.
fn format_duration(ms: u64) -> String {
    if ms < 60_000 {
        let secs = ms / 1000;
        format!("{}s", secs)
    } else {
        let minutes = ms / 60_000;
        let secs = (ms % 60_000) / 1000;
        if secs == 0 {
            format!("{}m", minutes)
        } else {
            format!("{}m {}s", minutes, secs)
        }
    }
}

/// Format a token count like the original: compact notation (e.g., "1.3k").
fn format_tokens(count: u64) -> String {
    if count >= 1_000 {
        let val = count as f64 / 1000.0;
        let formatted = format!("{:.1}k", val);
        // Remove ".0" like the original does
        formatted.replace(".0k", "k")
    } else {
        count.to_string()
    }
}

impl Widget for &SpinnerState {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if !self.active || area.height == 0 {
            return;
        }
        let frames = spinner_frames();
        let frame_char = frames[self.frame % frames.len()];
        let elapsed = self.elapsed_str();

        // The original renders: {spinner_glyph} {verb}... ({elapsed})
        // Color: spinner glyph in claude orange, verb text in claude orange,
        // parenthetical info in dim
        let verb = self.current_verb();
        let claude_color = Color::Rgb(215, 119, 87); // Claude orange

        let mut spans = vec![
            Span::styled(
                format!("{} ", frame_char),
                Style::default().fg(claude_color),
            ),
            Span::styled(format!("{}…", verb), Style::default().fg(claude_color)),
        ];

        // Duration and token info in parentheses, dim
        let mut info_parts = vec![elapsed];
        if self.tokens > 0 {
            info_parts.push(format!("{} tokens", format_tokens(self.tokens)));
        }
        if self.queued_count > 0 {
            info_parts.push(format!(
                "{} queued",
                self.queued_count
            ));
        }
        spans.push(Span::styled(
            format!(" ({})", info_parts.join(" · ")),
            Style::default().fg(Color::Rgb(153, 153, 153)),
        ));

        let line = Line::from(spans);
        buf.set_line(area.x, area.y, &line, area.width);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spinner_frames_bounce() {
        let frames = spinner_frames();
        // 6 forward + 6 reverse = 12
        assert_eq!(frames.len(), 12);
        assert_eq!(frames[0], "·");
        assert_eq!(frames[5], "✽");
        assert_eq!(frames[6], "✽");
        assert_eq!(frames[11], "·");
    }

    #[test]
    fn format_duration_seconds() {
        assert_eq!(format_duration(0), "0s");
        assert_eq!(format_duration(3500), "3s");
        assert_eq!(format_duration(59999), "59s");
    }

    #[test]
    fn format_duration_minutes() {
        assert_eq!(format_duration(60000), "1m");
        assert_eq!(format_duration(65000), "1m 5s");
        assert_eq!(format_duration(125000), "2m 5s");
    }

    #[test]
    fn format_tokens_compact() {
        assert_eq!(format_tokens(500), "500");
        assert_eq!(format_tokens(1000), "1k");
        assert_eq!(format_tokens(1300), "1.3k");
        assert_eq!(format_tokens(12500), "12.5k");
    }

    #[test]
    fn spinner_state_lifecycle() {
        let mut s = SpinnerState::new();
        assert!(!s.active);

        s.start(SpinnerMode::Thinking);
        assert!(s.active);
        assert_eq!(s.mode.label(), "Thinking");

        s.advance();
        assert_eq!(s.frame, 1);

        s.stop();
        assert!(!s.active);
    }
}
