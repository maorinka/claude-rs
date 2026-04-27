use claude_tui::markdown::*;
use claude_tui::widgets::message_list::*;
use ratatui::text::Line;

fn line_text(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<Vec<_>>()
        .join("")
}

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
fn test_markdown_ordered_list() {
    let lines = render_markdown("1. first\n2. second");
    assert_eq!(lines.len(), 2);
    assert!(line_text(&lines[0]).contains("1. first"));
}

#[test]
fn test_markdown_blockquote() {
    let lines = render_markdown("> quoted text");
    assert_eq!(lines.len(), 1);
    assert!(line_text(&lines[0]).contains("│ quoted text"));
}

#[test]
fn test_markdown_horizontal_rule() {
    let lines = render_markdown("---");
    assert_eq!(lines.len(), 1);
    assert!(line_text(&lines[0]).starts_with("───"));
}

#[test]
fn test_markdown_table() {
    let lines = render_markdown("| Name | Value |\n| --- | --- |\n| one | two |");
    assert_eq!(lines.len(), 2);
    assert!(line_text(&lines[0]).contains("Name"));
    assert!(line_text(&lines[1]).contains("two"));
}

#[test]
fn test_markdown_link() {
    let lines = render_markdown("See [docs](https://example.com).");
    assert_eq!(lines.len(), 1);
    let text = line_text(&lines[0]);
    assert!(text.contains("docs (https://example.com)"));
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
    assert_eq!(dialog.selected(), "always");
    dialog.next_button();
    assert_eq!(dialog.selected(), "deny");
    dialog.next_button();
    assert_eq!(dialog.selected(), "allow"); // wraps
    dialog.prev_button();
    assert_eq!(dialog.selected(), "deny");
}
