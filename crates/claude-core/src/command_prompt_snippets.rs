//! Short, self-contained command-prompt constants that don't
//! warrant their own module.
//!
//! Each item here is a verbatim port of TS source text; the
//! associated commands (or their feature infrastructure) aren't
//! fully wired in Rust, so the text is parked here for byte-
//! stable cache parity when they land.
//!
//! Contents:
//! - [`SESSION_NAME_GENERATION_SYSTEM_PROMPT`] — Haiku system
//!   prompt for auto-generating a kebab-case session name from
//!   conversation text. Port of TS
//!   `commands/rename/generateSessionName.ts:20-23`.
//! - [`STATUSLINE_DEFAULT_PROMPT`] + [`statusline_command_prompt`]
//!   — `/statusline` command prompt. Port of TS
//!   `commands/statusline.tsx:14-23`.
//! - [`moved_to_plugin_redirect`] — ant-only redirect message
//!   pointing users at `claude plugin install …`. Port of TS
//!   `commands/createMovedToPluginCommand.ts:44-57`.

use crate::tool_names::AGENT_TOOL_NAME;

/// Haiku system prompt for auto-generating a kebab-case session
/// name. Port of TS `commands/rename/generateSessionName.ts:20-23`.
/// Sent as the system-prompt line; the conversation text travels
/// as the user-prompt.
pub const SESSION_NAME_GENERATION_SYSTEM_PROMPT: &str = "Generate a short kebab-case name (2-4 words) that captures the main topic of this conversation. Use lowercase words separated by hyphens. Examples: \"fix-login-bug\", \"add-auth-feature\", \"refactor-api-client\", \"debug-test-failures\". Return JSON with a \"name\" field.";

/// Default prompt when `/statusline` is invoked with no args.
/// Port of TS `commands/statusline.tsx:14-15`.
pub const STATUSLINE_DEFAULT_PROMPT: &str =
    "Configure my statusLine from my shell PS1 configuration";

/// Build the `/statusline` command output. Port of TS
/// `commands/statusline.tsx:14-23`. Uses the current
/// `AGENT_TOOL_NAME` + `"statusline-setup"` subagent type.
///
/// `args` is the (trimmed) user arguments; empty strings fall
/// back to [`STATUSLINE_DEFAULT_PROMPT`].
pub fn statusline_command_prompt(args: &str) -> String {
    let prompt = if args.is_empty() {
        STATUSLINE_DEFAULT_PROMPT
    } else {
        args
    };
    format!("Create an {AGENT_TOOL_NAME} with subagent_type \"statusline-setup\" and the prompt \"{prompt}\"")
}

/// Ant-only redirect prompt for commands that moved to the
/// plugin marketplace. Port of TS
/// `commands/createMovedToPluginCommand.ts:44-57`.
///
/// Caller gates on `USER_TYPE === 'ant'` before emitting this;
/// `plugin_name` and `plugin_command` identify the target in the
/// marketplace.
pub fn moved_to_plugin_redirect(plugin_name: &str, plugin_command: &str) -> String {
    format!(
        "This command has been moved to a plugin. Tell the user:\n\n\
         1. To install the plugin, run:\n   \
         claude plugin install {plugin_name}@claude-code-marketplace\n\n\
         2. After installation, use /{plugin_name}:{plugin_command} to run this command\n\n\
         3. For more information, see: https://github.com/anthropics/claude-code-marketplace/blob/main/{plugin_name}/README.md\n\n\
         Do not attempt to run the command. Simply inform the user about the plugin installation."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_name_prompt_mentions_kebab_case_examples() {
        let p = SESSION_NAME_GENERATION_SYSTEM_PROMPT;
        assert!(p.contains("kebab-case"));
        assert!(p.contains("2-4 words"));
        assert!(p.contains("\"fix-login-bug\""));
        // Must demand JSON output with a `name` field.
        assert!(p.contains("Return JSON with a \"name\" field."));
    }

    #[test]
    fn statusline_default_prompt_when_args_empty() {
        let out = statusline_command_prompt("");
        assert!(out.contains(STATUSLINE_DEFAULT_PROMPT));
        assert!(out.contains(AGENT_TOOL_NAME));
        assert!(out.contains("subagent_type \"statusline-setup\""));
    }

    #[test]
    fn statusline_user_args_override_default() {
        let out = statusline_command_prompt("Use my oh-my-zsh theme");
        assert!(out.contains("Use my oh-my-zsh theme"));
        assert!(!out.contains(STATUSLINE_DEFAULT_PROMPT));
    }

    #[test]
    fn moved_to_plugin_redirect_references_marketplace() {
        let msg = moved_to_plugin_redirect("review", "review");
        assert!(msg.contains("claude plugin install review@claude-code-marketplace"));
        assert!(msg.contains("/review:review"));
        assert!(msg.contains("github.com/anthropics/claude-code-marketplace/blob/main/review/"));
        assert!(msg.ends_with("Do not attempt to run the command. Simply inform the user about the plugin installation."));
    }

    #[test]
    fn moved_to_plugin_handles_multi_word_commands() {
        let msg = moved_to_plugin_redirect("pr-tools", "review");
        assert!(msg.contains("pr-tools@claude-code-marketplace"));
        assert!(msg.contains("/pr-tools:review"));
    }
}
