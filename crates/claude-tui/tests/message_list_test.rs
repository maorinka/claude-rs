use claude_tui::markdown::*;
use claude_tui::widgets::message_list::*;

#[test]
fn test_message_list_push_and_len() {
    let mut list = MessageList::new();
    assert!(list.is_empty());
    list.push(MessageEntry::User {
        text: "hello".into(),
    });
    assert_eq!(list.len(), 1);
    list.push(MessageEntry::Assistant {
        text: "hi there".into(),
    });
    assert_eq!(list.len(), 2);
}

#[test]
fn test_message_list_clear() {
    let mut list = MessageList::new();
    list.push(MessageEntry::User {
        text: "test".into(),
    });
    list.clear();
    assert!(list.is_empty());
}

#[test]
fn test_message_list_scroll() {
    let mut list = MessageList::new();
    for i in 0..20 {
        list.push(MessageEntry::User {
            text: format!("msg {}", i),
        });
    }
    list.scroll_up(5);
    list.scroll_down(2);
    list.scroll_to_bottom();
}

#[test]
fn test_markdown_headers() {
    let lines = render_markdown("# Title\n## Subtitle\n### Section");
    assert_eq!(lines.len(), 3);
}

#[test]
fn test_markdown_code_block() {
    let lines = render_markdown("```\nfn main() {}\n```");
    assert_eq!(lines.len(), 3); // border + code + border
}

#[test]
fn test_markdown_list() {
    let lines = render_markdown("- item one\n- item two");
    assert_eq!(lines.len(), 2);
}

#[test]
fn test_markdown_inline_code() {
    let lines = render_markdown("Use `foo()` here");
    assert_eq!(lines.len(), 1);
    // Line should have multiple spans (text + code + text)
}

#[test]
fn test_markdown_bold() {
    let lines = render_markdown("This is **bold** text");
    assert_eq!(lines.len(), 1);
}

#[test]
fn test_permission_dialog_buttons() {
    use claude_tui::widgets::permission_dialog::*;
    let mut dialog =
        PermissionDialog::new("Bash".into(), "Execute command".into(), "ls -la".into());
    assert_eq!(dialog.selected(), "allow");
    dialog.next_button();
    assert_eq!(dialog.selected(), "deny");
    dialog.next_button();
    assert_eq!(dialog.selected(), "always");
    dialog.next_button();
    assert_eq!(dialog.selected(), "allow"); // wraps
    dialog.prev_button();
    assert_eq!(dialog.selected(), "always");
}
