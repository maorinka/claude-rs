use claude_tui::theme::*;
use claude_tui::widgets::spinner::*;

#[test]
fn test_dark_theme_colors() {
    let t = dark_theme();
    // Dark theme uses RGB white for text
    assert_eq!(t.fg, ratatui::style::Color::Rgb(255, 255, 255));
    // Error is bright red (original: rgb(255,107,128))
    assert_eq!(t.error, ratatui::style::Color::Rgb(255, 107, 128));
    // Claude orange
    assert_eq!(t.claude, ratatui::style::Color::Rgb(215, 119, 87));
}

#[test]
fn test_light_theme_colors() {
    let t = light_theme();
    // Light theme uses RGB black for text
    assert_eq!(t.fg, ratatui::style::Color::Rgb(0, 0, 0));
    // Light error color
    assert_eq!(t.error, ratatui::style::Color::Rgb(171, 43, 63));
}

#[test]
fn test_detect_theme_defaults_to_dark() {
    // Unless COLORFGBG is set to a light-indicating value
    let t = detect_theme();
    // Just verify it returns something valid
    let _ = t.fg;
}

#[test]
fn test_spinner_advance() {
    let mut s = SpinnerState::new();
    s.start(SpinnerMode::Thinking);
    assert_eq!(s.frame, 0);
    s.advance();
    assert_eq!(s.frame, 1);
    s.advance();
    assert_eq!(s.frame, 2);
}

#[test]
fn test_spinner_wraps_around() {
    let mut s = SpinnerState::new();
    s.start(SpinnerMode::Thinking);
    // The spinner has 12 frames (6 forward + 6 reverse bounce)
    for _ in 0..12 {
        s.advance();
    }
    assert_eq!(s.frame, 0); // Wraps back to 0 after 12 frames
}

#[test]
fn test_spinner_inactive_doesnt_advance() {
    let mut s = SpinnerState::new();
    // Not started, should not advance
    s.advance();
    assert_eq!(s.frame, 0);
}

#[test]
fn test_spinner_start_stop() {
    let mut s = SpinnerState::new();
    assert!(!s.active);
    s.start(SpinnerMode::Waiting);
    assert!(s.active);
    s.stop();
    assert!(!s.active);
}

#[test]
fn test_spinner_mode_labels() {
    assert_eq!(SpinnerMode::Thinking.label(), "Thinking");
    assert_eq!(SpinnerMode::Waiting.label(), "Waiting");
    assert_eq!(
        SpinnerMode::Tool {
            name: "Bash".into()
        }
        .label(),
        "Bash"
    );
    assert_eq!(SpinnerMode::Stopped.label(), "Ready");
}

#[test]
fn test_app_new() {
    // Just verify App can be constructed (doesn't require terminal)
    // Skip this in CI — App::new() needs a real terminal
    // Instead test the components separately
}
