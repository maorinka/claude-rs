//! `/debug` bundled-skill prompt.
//!
//! Port of TS `src/skills/bundled/debug.ts`. Helps the user
//! diagnose issues by reading the session debug log, settings
//! paths, and optionally handing off to the `claude-code-guide`
//! subagent. The Rust port ships the prompt template + helper
//! for rendering; the "tail the debug log" side-effects stay at
//! the skill-invocation site when the skill infrastructure is
//! wired up.
//!
//! TS baked-in literals (carried over verbatim):
//! - `DEFAULT_DEBUG_LINES_READ` → 20
//! - `CLAUDE_CODE_GUIDE_AGENT_TYPE` → `claude-code-guide`
//!
//! TS `justEnabledSection` is conditional — empty when debug
//! logging was already on, otherwise a 4-line notice. The
//! [`debug_just_enabled_section`] helper returns the right text
//! for either case.

/// Default number of tail lines the TS version pulls from the
/// debug log. Port of TS `debug.ts:9` `DEFAULT_DEBUG_LINES_READ`.
pub const DEBUG_LINES_READ: usize = 20;

/// Default tail-read budget in bytes. Port of TS `debug.ts:10`
/// `TAIL_READ_BYTES` (64 KiB).
pub const DEBUG_TAIL_READ_BYTES: u64 = 64 * 1024;

/// Fallback `logInfo` when the debug log does not exist yet.
/// Port of TS `debug.ts:55`.
pub const DEBUG_LOG_NO_FILE_YET: &str = "No debug log exists yet — logging was just enabled.";

/// Fallback `args` line when the user invoked `/debug` without a
/// description. Port of TS `debug.ts:83`.
pub const DEBUG_NO_ISSUE_DESCRIPTION: &str =
    "The user did not describe a specific issue. Read the debug log and summarize any errors, warnings, or notable issues.";

const DEBUG_PROMPT_TEMPLATE: &str = include_str!("prompts/debug_skill.md");

/// Build the "Debug Logging Just Enabled" section that appears
/// only when `enableDebugLogging()` flipped the flag for this
/// session. Port of TS `debug.ts:59-67`.
pub fn debug_just_enabled_section(was_already_logging: bool, debug_log_path: &str) -> String {
    if was_already_logging {
        String::new()
    } else {
        format!(
            "
## Debug Logging Just Enabled

Debug logging was OFF for this session until now. Nothing prior to this /debug invocation was captured.

Tell the user that debug logging is now active at `{debug_log_path}`, ask them to reproduce the issue, then re-read the log. If they can't reproduce, they can also restart with `claude --debug` to capture logs from startup.
"
        )
    }
}

/// Parameters for [`debug_prompt`].
pub struct DebugPromptInputs<'a> {
    /// True if debug logging was already on before the skill fired
    /// — suppresses the "just enabled" notice.
    pub was_already_logging: bool,
    /// Absolute path of the session debug log file.
    pub debug_log_path: &'a str,
    /// Pre-rendered log-tail section. Callers tail the log file
    /// themselves (Rust does not run the `open/stat/read` dance
    /// inline) and pass the resulting markdown block here. Use
    /// [`DEBUG_LOG_NO_FILE_YET`] when the file is missing.
    pub log_info: &'a str,
    /// User's issue description from skill args. Empty string ⇒
    /// [`DEBUG_NO_ISSUE_DESCRIPTION`] is substituted.
    pub issue_description: &'a str,
    /// `getSettingsFilePathForSource('userSettings')` equivalent.
    pub user_settings_path: &'a str,
    /// `getSettingsFilePathForSource('projectSettings')` equivalent.
    pub project_settings_path: &'a str,
    /// `getSettingsFilePathForSource('localSettings')` equivalent.
    pub local_settings_path: &'a str,
}

/// Render the full `/debug` skill prompt. Port of TS
/// `debug.ts:69-99` (the template literal + its four
/// interpolations).
pub fn debug_prompt(inputs: &DebugPromptInputs<'_>) -> String {
    let issue = if inputs.issue_description.is_empty() {
        DEBUG_NO_ISSUE_DESCRIPTION
    } else {
        inputs.issue_description
    };
    DEBUG_PROMPT_TEMPLATE
        .replace(
            "{{JUST_ENABLED_SECTION}}",
            &debug_just_enabled_section(inputs.was_already_logging, inputs.debug_log_path),
        )
        .replace("{{DEBUG_LOG_PATH}}", inputs.debug_log_path)
        .replace("{{LOG_INFO}}", inputs.log_info)
        .replace("{{ISSUE_DESCRIPTION}}", issue)
        .replace("{{USER_SETTINGS_PATH}}", inputs.user_settings_path)
        .replace("{{PROJECT_SETTINGS_PATH}}", inputs.project_settings_path)
        .replace("{{LOCAL_SETTINGS_PATH}}", inputs.local_settings_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_inputs<'a>() -> DebugPromptInputs<'a> {
        DebugPromptInputs {
            was_already_logging: false,
            debug_log_path: "/tmp/debug.log",
            log_info: "Log size: 1 KiB\n\n### Last 20 lines\n\n```\n[INFO] ok\n```",
            issue_description: "button doesn't work",
            user_settings_path: "/u/settings.json",
            project_settings_path: "/p/settings.json",
            local_settings_path: "/p/settings.local.json",
        }
    }

    #[test]
    fn header_and_canonical_sections_present() {
        let p = debug_prompt(&sample_inputs());
        assert!(p.starts_with("# Debug Skill"));
        assert!(p.contains("## Session Debug Log"));
        assert!(p.contains("## Issue Description"));
        assert!(p.contains("## Settings"));
        assert!(p.contains("## Instructions"));
    }

    #[test]
    fn just_enabled_notice_appears_when_flag_flipped() {
        let p = debug_prompt(&sample_inputs());
        assert!(p.contains("## Debug Logging Just Enabled"));
        assert!(p.contains("claude --debug"));
        assert!(p.contains("/tmp/debug.log"));
    }

    #[test]
    fn just_enabled_notice_suppressed_when_already_logging() {
        let mut inputs = sample_inputs();
        inputs.was_already_logging = true;
        let p = debug_prompt(&inputs);
        assert!(!p.contains("## Debug Logging Just Enabled"));
        assert!(!p.contains("claude --debug"));
    }

    #[test]
    fn all_settings_paths_interpolated() {
        let p = debug_prompt(&sample_inputs());
        assert!(p.contains("* user - /u/settings.json"));
        assert!(p.contains("* project - /p/settings.json"));
        assert!(p.contains("* local - /p/settings.local.json"));
    }

    #[test]
    fn fallback_issue_description_when_empty() {
        let mut inputs = sample_inputs();
        inputs.issue_description = "";
        let p = debug_prompt(&inputs);
        assert!(p.contains(DEBUG_NO_ISSUE_DESCRIPTION));
    }

    #[test]
    fn log_info_substituted_verbatim() {
        let p = debug_prompt(&sample_inputs());
        assert!(p.contains("Log size: 1 KiB"));
        assert!(p.contains("[INFO] ok"));
    }

    #[test]
    fn references_claude_code_guide_agent() {
        let p = debug_prompt(&sample_inputs());
        assert!(p.contains("claude-code-guide subagent"));
    }

    #[test]
    fn no_unsubstituted_slots_after_render() {
        let p = debug_prompt(&sample_inputs());
        assert!(!p.contains("{{DEBUG_LOG_PATH}}"));
        assert!(!p.contains("{{LOG_INFO}}"));
        assert!(!p.contains("{{ISSUE_DESCRIPTION}}"));
        assert!(!p.contains("{{USER_SETTINGS_PATH}}"));
        assert!(!p.contains("{{PROJECT_SETTINGS_PATH}}"));
        assert!(!p.contains("{{LOCAL_SETTINGS_PATH}}"));
        assert!(!p.contains("{{JUST_ENABLED_SECTION}}"));
    }
}
