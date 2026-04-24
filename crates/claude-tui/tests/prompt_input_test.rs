use claude_tui::widgets::prompt_input::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::empty())
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

#[test]
fn test_insert_characters() {
    let mut input = PromptInput::new();
    input.handle_key(key(KeyCode::Char('h')));
    input.handle_key(key(KeyCode::Char('i')));
    assert_eq!(input.text(), "hi");
    assert_eq!(input.cursor(), 2);
}

#[test]
fn test_backspace() {
    let mut input = PromptInput::new();
    input.handle_key(key(KeyCode::Char('a')));
    input.handle_key(key(KeyCode::Char('b')));
    input.handle_key(key(KeyCode::Backspace));
    assert_eq!(input.text(), "a");
}

#[test]
fn test_submit_clears_and_returns() {
    let mut input = PromptInput::new();
    input.handle_key(key(KeyCode::Char('h')));
    input.handle_key(key(KeyCode::Char('i')));
    match input.handle_key(key(KeyCode::Enter)) {
        InputAction::Submit(s) => assert_eq!(s, "hi"),
        _ => panic!("Expected Submit"),
    }
    assert!(input.is_empty());
}

#[test]
fn test_empty_enter_does_nothing() {
    let mut input = PromptInput::new();
    match input.handle_key(key(KeyCode::Enter)) {
        InputAction::None => {}
        _ => panic!("Expected None for empty enter"),
    }
}

#[test]
fn test_cursor_movement() {
    let mut input = PromptInput::new();
    input.handle_key(key(KeyCode::Char('a')));
    input.handle_key(key(KeyCode::Char('b')));
    input.handle_key(key(KeyCode::Char('c')));
    input.handle_key(key(KeyCode::Left));
    input.handle_key(key(KeyCode::Left));
    assert_eq!(input.cursor(), 1);
    input.handle_key(key(KeyCode::Right));
    assert_eq!(input.cursor(), 2);
}

#[test]
fn test_ctrl_a_e() {
    let mut input = PromptInput::new();
    input.handle_key(key(KeyCode::Char('a')));
    input.handle_key(key(KeyCode::Char('b')));
    input.handle_key(ctrl('a'));
    assert_eq!(input.cursor(), 0);
    input.handle_key(ctrl('e'));
    assert_eq!(input.cursor(), 2);
}

#[test]
fn test_ctrl_k_kill_to_end() {
    let mut input = PromptInput::new();
    for c in "hello".chars() {
        input.handle_key(key(KeyCode::Char(c)));
    }
    input.handle_key(ctrl('a'));
    input.handle_key(key(KeyCode::Right));
    input.handle_key(key(KeyCode::Right));
    input.handle_key(ctrl('k'));
    assert_eq!(input.text(), "he");
}

#[test]
fn test_history_navigation() {
    let mut input = PromptInput::new();
    // Submit two entries
    for c in "first".chars() {
        input.handle_key(key(KeyCode::Char(c)));
    }
    input.handle_key(key(KeyCode::Enter));
    for c in "second".chars() {
        input.handle_key(key(KeyCode::Char(c)));
    }
    input.handle_key(key(KeyCode::Enter));

    // Navigate up
    input.handle_key(key(KeyCode::Up));
    assert_eq!(input.text(), "second");
    input.handle_key(key(KeyCode::Up));
    assert_eq!(input.text(), "first");
    // Navigate back down
    input.handle_key(key(KeyCode::Down));
    assert_eq!(input.text(), "second");
    input.handle_key(key(KeyCode::Down));
    assert!(input.is_empty()); // Back to current
}
