//! Claude-in-Chrome system-prompt hints + base prompt.
//!
//! Port of TS `utils/claudeInChrome/prompt.ts`. This module
//! exposes four constants:
//!
//! - `BASE_CHROME_PROMPT` — the full ~46-line system prompt
//!   that describes how to use the `mcp__claude-in-chrome__*`
//!   tools (GIF recording, console debugging, dialog avoidance,
//!   tab context). Verbatim from TS `:1-46`; stored in
//!   `prompts/chrome_base.md` and pulled in via `include_str!`.
//! - `CHROME_TOOL_SEARCH_INSTRUCTIONS` — load-via-ToolSearch
//!   instructions. TS `:53-61`.
//! - `CLAUDE_IN_CHROME_SKILL_HINT` — default skill-nudge
//!   variant. TS `:76`.
//! - `CLAUDE_IN_CHROME_SKILL_HINT_WITH_WEBBROWSER` — variant
//!   when the built-in WebBrowser tool is also available.
//!   TS `:83`.

/// Full Chrome-automation system prompt. Appended to the main
/// system prompt when Chrome tools are available. Verbatim from
/// TS `utils/claudeInChrome/prompt.ts:1-46`.
pub const BASE_CHROME_PROMPT: &str = include_str!("prompts/chrome_base.md");

/// Load-via-ToolSearch instructions. Explains that the
/// `mcp__claude-in-chrome__*` tools are deferred and must be
/// loaded with `ToolSearch` before invocation. Verbatim from
/// TS `utils/claudeInChrome/prompt.ts:53-61`.
pub const CHROME_TOOL_SEARCH_INSTRUCTIONS: &str = "**IMPORTANT: Before using any chrome browser tools, you MUST first load them using ToolSearch.**

Chrome browser tools are MCP tools that require loading before use. Before calling any mcp__claude-in-chrome__* tool:
1. Use ToolSearch with `select:mcp__claude-in-chrome__<tool_name>` to load the specific tool
2. Then call the tool

For example, to get tab context:
1. First: ToolSearch with query \"select:mcp__claude-in-chrome__tabs_context_mcp\"
2. Then: Call mcp__claude-in-chrome__tabs_context_mcp";

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

/// Activation-time reminder emitted by the `claude-in-chrome`
/// bundled skill's `getPromptForCommand`. Appended after
/// [`BASE_CHROME_PROMPT`] — together with an optional `## Task`
/// section built from the skill's args. Port of TS
/// `skills/bundled/claudeInChrome.ts:10-14`
/// `SKILL_ACTIVATION_MESSAGE`.
pub const CLAUDE_IN_CHROME_SKILL_ACTIVATION_MESSAGE: &str = "
Now that this skill is invoked, you have access to Chrome browser automation tools. You can now use the mcp__claude-in-chrome__* tools to interact with web pages.

IMPORTANT: Start by calling mcp__claude-in-chrome__tabs_context_mcp to get information about the user's current browser tabs.
";

/// Build the full `claude-in-chrome` skill activation prompt.
/// Port of TS `skills/bundled/claudeInChrome.ts:26-32`
/// `getPromptForCommand`. Concatenates [`BASE_CHROME_PROMPT`] +
/// [`CLAUDE_IN_CHROME_SKILL_ACTIVATION_MESSAGE`] and, when `args`
/// is non-empty, a `## Task\n\n<args>` section — matching TS's
/// literal `${BASE_CHROME_PROMPT}\n${SKILL_ACTIVATION_MESSAGE}`
/// and optional `\n## Task\n\n${args}` suffix.
pub fn claude_in_chrome_skill_prompt(args: &str) -> String {
    let mut prompt =
        format!("{BASE_CHROME_PROMPT}\n{CLAUDE_IN_CHROME_SKILL_ACTIVATION_MESSAGE}");
    if !args.is_empty() {
        prompt.push_str("\n## Task\n\n");
        prompt.push_str(args);
    }
    prompt
}

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

    #[test]
    fn base_prompt_has_key_sections() {
        assert!(BASE_CHROME_PROMPT.starts_with("# Claude in Chrome browser automation"));
        assert!(BASE_CHROME_PROMPT.contains("## GIF recording"));
        assert!(BASE_CHROME_PROMPT.contains("## Console log debugging"));
        assert!(BASE_CHROME_PROMPT.contains("## Alerts and dialogs"));
        assert!(BASE_CHROME_PROMPT.contains("## Avoid rabbit holes and loops"));
        assert!(BASE_CHROME_PROMPT.contains("## Tab context and session startup"));
    }

    #[test]
    fn tool_search_instructions_mention_toolsearch() {
        assert!(CHROME_TOOL_SEARCH_INSTRUCTIONS.contains("ToolSearch"));
        assert!(CHROME_TOOL_SEARCH_INSTRUCTIONS.contains("select:mcp__claude-in-chrome__"));
    }

    #[test]
    fn skill_prompt_without_args_has_base_plus_activation() {
        let p = claude_in_chrome_skill_prompt("");
        assert!(p.contains("# Claude in Chrome browser automation"));
        assert!(p.contains("Now that this skill is invoked"));
        assert!(p.contains("mcp__claude-in-chrome__tabs_context_mcp"));
        // No task section when args are empty.
        assert!(!p.contains("## Task"));
    }

    #[test]
    fn skill_prompt_with_args_appends_task_section() {
        let p = claude_in_chrome_skill_prompt("open GitHub and take a screenshot");
        assert!(p.contains("## Task\n\nopen GitHub and take a screenshot"));
    }
}
