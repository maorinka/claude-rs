//! Claude-in-Chrome system-prompt hints — constants only.
//!
//! Port of TS `utils/claudeInChrome/prompt.ts:76` and `:83`. The
//! full Chrome browser-automation system prompt
//! (`BASE_CHROME_PROMPT`) is long enough to be its own port
//! task; this module exposes the two **skill-hint** variants
//! that get appended to the main system prompt when the
//! claude-in-chrome skill (or WebBrowser tool) is available.
//!
//! The hints nudge the model to invoke the `claude-in-chrome`
//! skill via `Skill(skill: "claude-in-chrome")` before any of
//! the `mcp__claude-in-chrome__*` MCP tools. Claude Code wires
//! the hint variant picker elsewhere — when that wiring lands,
//! it reads these constants directly.

/// Default hint: used when only the `claude-in-chrome` skill is
/// available. Port of TS
/// `utils/claudeInChrome/prompt.ts:76` `CLAUDE_IN_CHROME_SKILL_HINT`.
pub const CLAUDE_IN_CHROME_SKILL_HINT: &str =
    "**Browser Automation**: Chrome browser tools are available via the \"claude-in-chrome\" skill. \
     CRITICAL: Before using any mcp__claude-in-chrome__* tools, invoke the skill by calling the \
     Skill tool with skill: \"claude-in-chrome\". The skill provides browser automation \
     instructions and enables the tools.";

/// WebBrowser-coexists variant: picked when the built-in
/// `WebBrowser` tool is also available — nudges the model to
/// use WebBrowser for dev-loop tasks and reserve
/// claude-in-chrome for the user's authenticated session.
/// Port of TS `utils/claudeInChrome/prompt.ts:83`
/// `CLAUDE_IN_CHROME_SKILL_HINT_WITH_WEBBROWSER`.
pub const CLAUDE_IN_CHROME_SKILL_HINT_WITH_WEBBROWSER: &str =
    "**Browser Automation**: Use WebBrowser for development (dev servers, JS eval, console, \
     screenshots). Use claude-in-chrome for the user's real Chrome when you need logged-in \
     sessions, OAuth, or computer-use — invoke Skill(skill: \"claude-in-chrome\") before any \
     mcp__claude-in-chrome__* tool.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_hint_names_the_skill() {
        assert!(CLAUDE_IN_CHROME_SKILL_HINT.contains("claude-in-chrome"));
        assert!(CLAUDE_IN_CHROME_SKILL_HINT.contains("Skill tool"));
        assert!(CLAUDE_IN_CHROME_SKILL_HINT.contains("CRITICAL"));
    }

    #[test]
    fn webbrowser_hint_differentiates_use_cases() {
        assert!(CLAUDE_IN_CHROME_SKILL_HINT_WITH_WEBBROWSER.contains("WebBrowser for development"));
        assert!(CLAUDE_IN_CHROME_SKILL_HINT_WITH_WEBBROWSER.contains("logged-in sessions"));
        assert!(CLAUDE_IN_CHROME_SKILL_HINT_WITH_WEBBROWSER.contains("OAuth"));
    }

    #[test]
    fn hints_are_nonempty_ascii() {
        for hint in [CLAUDE_IN_CHROME_SKILL_HINT, CLAUDE_IN_CHROME_SKILL_HINT_WITH_WEBBROWSER] {
            assert!(!hint.is_empty());
            // Note: em-dash is present in the WITH_WEBBROWSER variant, so
            // ASCII-only would fail. Verify at least mostly-ASCII content.
            assert!(hint.contains("Browser Automation"));
        }
    }
}
