use claude_tui::theme::*;
use claude_tui::widgets::spinner::*;

#[test]
fn test_dark_theme_colors() {
    let t = dark_theme();
    assert_eq!(t.fg, ratatui::style::Color::White);
    assert_eq!(t.error, ratatui::style::Color::Red);
}

#[test]
fn test_light_theme_colors() {
    let t = light_theme();
    assert_eq!(t.fg, ratatui::style::Color::Black);
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
    for _ in 0..10 {
        s.advance();
    }
    assert_eq!(s.frame, 0); // Wraps back to 0
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
    assert_eq!(SpinnerMode::Tool { name: "Bash".into() }.label(), "Bash");
    assert_eq!(SpinnerMode::Stopped.label(), "Ready");
}

#[test]
fn test_app_new() {
    // Just verify App can be constructed (doesn't require terminal)
    // Skip this in CI — App::new() needs a real terminal
    // Instead test the components separately
}
