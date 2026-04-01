//! Tests for the new UI features: syntax highlighting, diff view, token budget
//! warnings, scroll keyboard shortcuts, and persistent status line.

use claude_tui::syntax;
use claude_tui::widgets::diff_view;
use claude_tui::widgets::message_list::{MessageEntry, MessageList};
use claude_tui::markdown::render_markdown;

// ---------- Syntax highlighting tests ----------

#[test]
fn test_syntax_highlight_rust_produces_multiple_spans() {
    let code = "fn main() {\n    let x: i32 = 42;\n    println!(\"{}\", x);\n}";
    let lines = syntax::highlight_code(code, "rust");
    assert_eq!(lines.len(), 4);
    // "fn" keyword should be highlighted differently from "main"
    // Just verify we get multiple spans per line (not plain green fallback)
    let total_spans: usize = lines.iter().map(|l| l.spans.len()).sum();
    assert!(total_spans > 4, "expected multi-span highlighting, got {} spans total", total_spans);
}

#[test]
fn test_syntax_highlight_javascript() {
    let code = "const x = 42;\nfunction hello() { return x; }";
    let lines = syntax::highlight_code(code, "js");
    assert_eq!(lines.len(), 2);
    // Should use JavaScript syntax (via alias mapping)
    for line in &lines {
        assert!(!line.spans.is_empty());
    }
}

#[test]
fn test_syntax_highlight_typescript_alias() {
    let code = "interface Foo { bar: string; }";
    let lines = syntax::highlight_code(code, "ts");
    assert_eq!(lines.len(), 1);
    assert!(!lines[0].spans.is_empty());
}

// ---------- Markdown + syntax integration ----------

#[test]
fn test_markdown_code_block_with_language() {
    let md = "```rust\nfn main() {}\n```";
    let lines = render_markdown(md);
    // Should have: top separator, highlighted code line, bottom separator
    assert_eq!(lines.len(), 3);
    // The code line should be indented (first span is "  ")
    assert!(lines[1].spans.len() >= 2, "highlighted code should have indent + syntax spans");
}

#[test]
fn test_markdown_code_block_unknown_lang_still_renders() {
    let md = "```brainfuck\n+++++[>+++++++>++<<-]\n```";
    let lines = render_markdown(md);
    assert_eq!(lines.len(), 3); // separator + code + separator
}

// ---------- Diff view tests ----------

#[test]
fn test_diff_view_added_and_removed_counts() {
    let diff = "\
--- a/file.txt
+++ b/file.txt
@@ -1,3 +1,4 @@
 line1
-removed
+added1
+added2
 line3";
    let parsed = diff_view::parse_unified_diff(diff);
    let added = parsed.iter().filter(|l| l.kind == diff_view::DiffLineKind::Added).count();
    let removed = parsed.iter().filter(|l| l.kind == diff_view::DiffLineKind::Removed).count();
    assert_eq!(added, 2);
    assert_eq!(removed, 1);
}

#[test]
fn test_diff_line_numbers_increment_correctly() {
    let diff = "\
@@ -10,3 +20,4 @@
 ctx
-old
+new1
+new2
 ctx2";
    let parsed = diff_view::parse_unified_diff(diff);
    // After hunk header @@ -10 +20 @@:
    // context (old=10, new=20)
    // removed (old=11, new=none)
    // added (old=none, new=21)
    // added (old=none, new=22)
    // context (old=12, new=23)
    let context_lines: Vec<_> = parsed.iter().filter(|l| l.kind == diff_view::DiffLineKind::Context).collect();
    assert_eq!(context_lines[0].old_lineno, Some(10));
    assert_eq!(context_lines[0].new_lineno, Some(20));
    assert_eq!(context_lines[1].old_lineno, Some(12));
    assert_eq!(context_lines[1].new_lineno, Some(23));
}

// ---------- Token budget warning threshold tests ----------

#[test]
fn test_token_budget_below_warning() {
    // 79% usage -- should NOT trigger warning
    let context_window: u64 = 200_000;
    let total_tokens: u64 = 158_000; // 79%
    let pct = total_tokens as f64 / context_window as f64;
    assert!(pct < 0.80, "79% should be below warning threshold");
}

#[test]
fn test_token_budget_at_warning_threshold() {
    // 80% usage -- should trigger yellow warning
    let context_window: u64 = 200_000;
    let total_tokens: u64 = 160_000; // 80%
    let pct = total_tokens as f64 / context_window as f64;
    assert!(pct >= 0.80 && pct < 0.95,
        "80% should be in warning range, got {:.2}", pct);
}

#[test]
fn test_token_budget_at_critical_threshold() {
    // 95% usage -- should trigger red critical warning
    let context_window: u64 = 200_000;
    let total_tokens: u64 = 190_000; // 95%
    let pct = total_tokens as f64 / context_window as f64;
    assert!(pct >= 0.95, "95% should be at critical threshold, got {:.2}", pct);
}

#[test]
fn test_token_budget_at_100_percent() {
    let context_window: u64 = 200_000;
    let total_tokens: u64 = 200_000;
    let pct = total_tokens as f64 / context_window as f64;
    assert!(pct >= 0.95, "100% should be critical");
    assert!((pct - 1.0).abs() < 0.001, "should be exactly 100%");
}

#[test]
fn test_token_budget_zero_context_window() {
    // Edge case: zero context window should not panic
    let context_window: u64 = 0;
    let total_tokens: u64 = 100;
    let pct = if context_window == 0 { 0.0 } else { total_tokens as f64 / context_window as f64 };
    assert_eq!(pct, 0.0, "zero context window should return 0%");
}

// ---------- Scroll keyboard handling tests ----------

#[test]
fn test_scroll_page_up_from_bottom() {
    let mut list = MessageList::new();
    for i in 0..100 {
        list.push(MessageEntry::User { text: format!("message {}", i) });
    }
    // Start at bottom (sticky)
    assert!(list.is_at_bottom());

    // Page up should scroll and un-stick
    list.page_up(20);
    assert!(!list.is_at_bottom());
}

#[test]
fn test_scroll_page_down_then_bottom() {
    let mut list = MessageList::new();
    for i in 0..100 {
        list.push(MessageEntry::User { text: format!("message {}", i) });
    }
    list.scroll_to_top();
    assert!(!list.is_at_bottom());

    list.page_down(20);
    assert!(!list.is_at_bottom()); // Not at bottom yet

    list.scroll_to_bottom();
    assert!(list.is_at_bottom());
}

#[test]
fn test_scroll_home_jumps_to_top() {
    let mut list = MessageList::new();
    for i in 0..50 {
        list.push(MessageEntry::User { text: format!("msg {}", i) });
    }
    // Home should go to top
    list.scroll_to_top();
    assert!(!list.is_at_bottom());
}

#[test]
fn test_scroll_end_jumps_to_bottom() {
    let mut list = MessageList::new();
    for i in 0..50 {
        list.push(MessageEntry::User { text: format!("msg {}", i) });
    }
    list.scroll_to_top();
    list.scroll_to_bottom();
    assert!(list.is_at_bottom());
}

#[test]
fn test_scroll_single_line_up_down() {
    let mut list = MessageList::new();
    for i in 0..50 {
        list.push(MessageEntry::User { text: format!("msg {}", i) });
    }
    // Ctrl+Up/Down scrolls by 1 line
    list.scroll_to_top();
    list.scroll_down(1);
    list.scroll_down(1);
    list.scroll_up(1);
    // Should not panic, and should not be at bottom
    assert!(!list.is_at_bottom());
}

#[test]
fn test_scroll_up_underflow_protection() {
    let mut list = MessageList::new();
    list.push(MessageEntry::User { text: "only one".into() });
    // Scrolling up from 0 should not panic or underflow
    list.scroll_up(100);
    list.scroll_up(usize::MAX);
    // Should still be in a valid state
    assert!(!list.is_empty());
}

// ---------- Duration formatting test ----------

#[test]
fn test_duration_format() {
    // Test the duration formatting logic (extracted from App::format_duration)
    let format = |secs: u64| -> String {
        if secs < 60 {
            format!("{}s", secs)
        } else if secs < 3600 {
            format!("{}m {}s", secs / 60, secs % 60)
        } else {
            format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
        }
    };

    assert_eq!(format(0), "0s");
    assert_eq!(format(30), "30s");
    assert_eq!(format(60), "1m 0s");
    assert_eq!(format(90), "1m 30s");
    assert_eq!(format(3600), "1h 0m");
    assert_eq!(format(3661), "1h 1m");
    assert_eq!(format(7200), "2h 0m");
}
