use claude_tui::markdown::render_markdown;
use ratatui::style::Modifier;

#[test]
fn test_unclosed_bold_treated_as_literal() {
    // "**bold text" with no closing ** should render the ** as literal text
    let lines = render_markdown("**unclosed bold");
    assert_eq!(lines.len(), 1);

    // Collect all text from spans
    let full_text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
    assert!(
        full_text.contains("**"),
        "Unclosed ** should appear as literal text, got: {:?}",
        full_text
    );
    assert!(
        full_text.contains("unclosed bold"),
        "Content after ** should be preserved, got: {:?}",
        full_text
    );

    // No span should have BOLD modifier (since ** was never closed)
    for span in &lines[0].spans {
        assert!(
            !span.style.add_modifier.contains(Modifier::BOLD),
            "No span should be bold when ** is unclosed, but found bold span: {:?}",
            span
        );
    }
}

#[test]
fn test_closed_bold_renders_as_bold() {
    let lines = render_markdown("**bold text**");
    assert_eq!(lines.len(), 1);

    // There should be at least one bold span
    let has_bold = lines[0].spans.iter().any(|s| {
        s.style.add_modifier.contains(Modifier::BOLD) && s.content.as_ref() == "bold text"
    });
    assert!(has_bold, "Closed ** should render as bold, spans: {:?}", lines[0].spans);
}

#[test]
fn test_unclosed_bold_with_other_text() {
    let lines = render_markdown("hello **unclosed bold");
    assert_eq!(lines.len(), 1);

    let full_text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
    assert!(
        full_text.contains("hello"),
        "Text before ** should be preserved"
    );
    assert!(
        full_text.contains("**unclosed bold") || (full_text.contains("**") && full_text.contains("unclosed bold")),
        "Unclosed ** should appear as literal text"
    );
}
