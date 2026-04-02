use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, ThemeSet};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;

/// Thread-local syntax highlighting state (SyntaxSet + ThemeSet are expensive
/// to construct but can be reused across calls).
struct SyntaxState {
    syntax_set: SyntaxSet,
    theme_set: ThemeSet,
}

impl SyntaxState {
    fn new() -> Self {
        Self {
            syntax_set: SyntaxSet::load_defaults_newlines(),
            theme_set: ThemeSet::load_defaults(),
        }
    }
}

thread_local! {
    static SYNTAX: SyntaxState = SyntaxState::new();
}

/// Map a syntect RGBA color to the nearest ratatui Color.
fn syntect_to_ratatui_color(c: syntect::highlighting::Color) -> Color {
    Color::Rgb(c.r, c.g, c.b)
}

/// Map a code fence language label to a syntect syntax name.
/// Returns the input as-is when no special mapping is needed.
fn map_language(lang: &str) -> &str {
    match lang.to_lowercase().as_str() {
        "js" | "jsx" => "JavaScript",
        "ts" | "tsx" => "TypeScript",
        "py" => "Python",
        "rb" => "Ruby",
        "rs" => "Rust",
        "sh" | "bash" | "zsh" => "Bourne Again Shell (bash)",
        "yml" => "YAML",
        "md" | "markdown" => "Markdown",
        "cs" | "csharp" => "C#",
        "cpp" | "c++" | "cxx" | "cc" => "C++",
        "hs" => "Haskell",
        "kt" | "kotlin" => "Kotlin",
        "tf" | "hcl" => "HCL",
        "dockerfile" => "Dockerfile",
        "makefile" | "make" => "Makefile",
        "toml" => "TOML",
        other => {
            // Return the original &str (lifetime matches input)
            // The caller will try a case-insensitive lookup in syntect
            // This is a fallback for the borrow checker - we return the
            // original slice.
            let _ = other;
            lang
        }
    }
}

/// Highlight a block of source code and return styled ratatui `Line`s.
///
/// `language` should be the token from the code fence (e.g. "rust", "python").
/// If the language is not recognized, a plain green fallback is used.
pub fn highlight_code(code: &str, language: &str) -> Vec<Line<'static>> {
    SYNTAX.with(|state| {
        let mapped = map_language(language);

        // Try to find syntax by name or extension
        let syntax = state
            .syntax_set
            .find_syntax_by_token(mapped)
            .or_else(|| state.syntax_set.find_syntax_by_extension(language))
            .or_else(|| {
                state
                    .syntax_set
                    .find_syntax_by_extension(&language.to_lowercase())
            });

        let syntax = match syntax {
            Some(s) => s,
            None => {
                // Fallback: plain green monospace style
                return code
                    .lines()
                    .map(|line| {
                        Line::from(Span::styled(
                            line.to_string(),
                            Style::default().fg(Color::Green),
                        ))
                    })
                    .collect();
            }
        };

        let theme = &state.theme_set.themes["base16-ocean.dark"];
        let mut highlighter = HighlightLines::new(syntax, theme);
        let mut lines = Vec::new();

        for line in LinesWithEndings::from(code) {
            let ranges = highlighter
                .highlight_line(line, &state.syntax_set)
                .unwrap_or_default();

            let spans: Vec<Span<'static>> = ranges
                .into_iter()
                .map(|(style, text)| {
                    let fg = syntect_to_ratatui_color(style.foreground);
                    let mut rat_style = Style::default().fg(fg);
                    if style.font_style.contains(FontStyle::BOLD) {
                        rat_style = rat_style.add_modifier(ratatui::style::Modifier::BOLD);
                    }
                    if style.font_style.contains(FontStyle::ITALIC) {
                        rat_style = rat_style.add_modifier(ratatui::style::Modifier::ITALIC);
                    }
                    if style.font_style.contains(FontStyle::UNDERLINE) {
                        rat_style = rat_style.add_modifier(ratatui::style::Modifier::UNDERLINED);
                    }
                    Span::styled(text.trim_end_matches('\n').to_string(), rat_style)
                })
                .collect();

            lines.push(Line::from(spans));
        }

        lines
    })
}

/// Detect the language from a code fence opening line.
/// Input: the full line starting with ``` (e.g. "```rust" or "```python3")
/// Returns the language token or None if it's just plain ```.
pub fn detect_language(fence_line: &str) -> Option<String> {
    let trimmed = fence_line.trim();
    let after_backticks = trimmed.trim_start_matches('`');
    if after_backticks.is_empty() {
        return None;
    }
    // Take the first word (handles "```rust ignore" or "```python3")
    let lang = after_backticks
        .split_whitespace()
        .next()
        .map(|s| s.to_lowercase());
    lang.filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_highlight_rust_code() {
        let code = "fn main() {\n    println!(\"hello\");\n}";
        let lines = highlight_code(code, "rust");
        assert_eq!(lines.len(), 3);
        // Each line should have at least one span
        for line in &lines {
            assert!(!line.spans.is_empty(), "line should have spans");
        }
    }

    #[test]
    fn test_highlight_python_code() {
        let code = "def hello():\n    print('world')";
        let lines = highlight_code(code, "python");
        assert_eq!(lines.len(), 2);
        for line in &lines {
            assert!(!line.spans.is_empty());
        }
    }

    #[test]
    fn test_highlight_unknown_language_fallback() {
        let code = "some text\nanother line";
        let lines = highlight_code(code, "nonexistent_language_xyz");
        assert_eq!(lines.len(), 2);
        // Fallback should be green
        for line in &lines {
            assert_eq!(line.spans.len(), 1);
            assert_eq!(line.spans[0].style.fg, Some(Color::Green));
        }
    }

    #[test]
    fn test_detect_language_rust() {
        assert_eq!(detect_language("```rust"), Some("rust".to_string()));
    }

    #[test]
    fn test_detect_language_python_with_extras() {
        assert_eq!(
            detect_language("```python ignore"),
            Some("python".to_string())
        );
    }

    #[test]
    fn test_detect_language_bare_fence() {
        assert_eq!(detect_language("```"), None);
    }

    #[test]
    fn test_detect_language_js_alias() {
        assert_eq!(detect_language("```js"), Some("js".to_string()));
    }

    #[test]
    fn test_map_language_aliases() {
        assert_eq!(map_language("js"), "JavaScript");
        assert_eq!(map_language("ts"), "TypeScript");
        assert_eq!(map_language("py"), "Python");
        assert_eq!(map_language("rs"), "Rust");
        assert_eq!(map_language("sh"), "Bourne Again Shell (bash)");
    }

    #[test]
    fn test_highlight_produces_colored_spans() {
        let code = "let x: i32 = 42;";
        let lines = highlight_code(code, "rust");
        assert_eq!(lines.len(), 1);
        // Should have multiple colored spans (keywords, literals, etc.)
        let has_color = lines[0].spans.iter().any(|span| span.style.fg.is_some());
        assert!(has_color, "highlighted code should have colored spans");
    }

    #[test]
    fn test_highlight_empty_code() {
        let lines = highlight_code("", "rust");
        // Empty input produces no lines
        assert!(lines.is_empty());
    }

    #[test]
    fn test_highlight_multiline_with_line_count() {
        let code = "use std::io;\nfn main() {\n    let x = 1;\n    let y = 2;\n}";
        let lines = highlight_code(code, "rust");
        assert_eq!(lines.len(), 5);
    }
}
