//! `/schedule` (schedule-remote-agents) bundled-skill prompt.
//!
//! Port of TS `src/skills/bundled/scheduleRemoteAgents.ts`.
//! Helps the user create, update, list, or run remote triggers
//! that spawn fully isolated CCR sessions in Anthropic's cloud
//! on a cron schedule.
//!
//! # Scope
//!
//! The TS skill's buildPrompt() stitches together many pieces:
//! - user's local timezone
//! - connected claude.ai MCP connectors (sanitized)
//! - current git repo HTTPS URL (from `getRemoteUrl`)
//! - list of fetched cloud environments (+ a just-created one
//!   when the user had none)
//! - setup notes (heads-up block for missing prerequisites)
//! - GitHub-access reminder (gated by a GrowthBook flag)
//! - user args from the skill invocation
//!
//! None of those sources are implemented in the Rust port yet.
//! This module ports the prompt *template* + the text helpers
//! (setup-notes formatting, connector sanitization, MCP id
//! decoding, first-step selection) so callers can assemble the
//! prompt as the infra lands. [`schedule_remote_agents_prompt`]
//! takes a fully-prepared inputs struct.

const SCHEDULE_PROMPT_TEMPLATE: &str = include_str!("prompts/schedule_remote_agents.md");

/// Fixed question body shown in the initial AskUserQuestion
/// dialog. Port of TS `BASE_QUESTION` (scheduleRemoteAgents.ts:111).
pub const SCHEDULE_BASE_QUESTION: &str =
    "What would you like to do with scheduled remote agents?";

/// Fallback shown in the connectors section when the user has no
/// claude.ai MCP connectors connected. Port of TS
/// `formatConnectorsInfo` empty-list path
/// (scheduleRemoteAgents.ts:99).
pub const SCHEDULE_NO_CONNECTORS_MESSAGE: &str =
    "No connected MCP connectors found. The user may need to connect servers at https://claude.ai/settings/connectors";

/// Sanitize a connector name to the `[a-zA-Z0-9_-]` character
/// class the MCP-connection body accepts. Port of TS
/// `sanitizeConnectorName` (scheduleRemoteAgents.ts:89-95).
pub fn sanitize_connector_name(name: &str) -> String {
    // Strip a leading "claude.ai-" / "claude ai " / "claude.ai." prefix
    // (case-insensitive). Match the TS regex
    // `/^claude[.\s-]ai[.\s-]/i`.
    let lower = name.to_lowercase();
    let stripped = if let Some(idx) = strip_claude_ai_prefix(&lower) {
        &name[idx..]
    } else {
        name
    };
    // Replace any run of non-[a-zA-Z0-9_-] with a single dash.
    let mut out = String::with_capacity(stripped.len());
    let mut last_was_dash = false;
    for c in stripped.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            out.push(c);
            last_was_dash = c == '-';
        } else if !last_was_dash {
            out.push('-');
            last_was_dash = true;
        }
    }
    // Collapse any remaining runs of dashes (paranoid) and trim.
    while out.contains("--") {
        out = out.replace("--", "-");
    }
    out.trim_matches('-').to_string()
}

fn strip_claude_ai_prefix(lower: &str) -> Option<usize> {
    // Matches `claude[.\s-]ai[.\s-]` at the start.
    if !lower.starts_with("claude") {
        return None;
    }
    let after_claude = &lower[6..];
    let sep1 = after_claude.chars().next()?;
    if !matches!(sep1, '.' | '-' | ' ' | '\t') {
        return None;
    }
    let after_sep1 = &after_claude[sep1.len_utf8()..];
    if !after_sep1.starts_with("ai") {
        return None;
    }
    let after_ai = &after_sep1[2..];
    let sep2 = after_ai.chars().next()?;
    if !matches!(sep2, '.' | '-' | ' ' | '\t') {
        return None;
    }
    // Total prefix length: "claude" + sep1 + "ai" + sep2 bytes.
    Some(6 + sep1.len_utf8() + 2 + sep2.len_utf8())
}

/// Format a heads-up bulleted list for setup notes. Port of TS
/// `formatSetupNotes` (scheduleRemoteAgents.ts:118-121).
pub fn format_setup_notes(notes: &[&str]) -> String {
    let items: Vec<String> = notes.iter().map(|n| format!("- {n}")).collect();
    format!("⚠ Heads-up:\n{}", items.join("\n"))
}

/// Fields the caller pre-computes before rendering the prompt.
/// Every field is a caller-produced string — the module doesn't
/// touch env/git/API state.
pub struct ScheduleRemoteAgentsInputs<'a> {
    /// User's local timezone string, e.g. `"America/Los_Angeles"`.
    pub user_timezone: &'a str,
    /// Pre-rendered connectors block (bullet list or the
    /// [`SCHEDULE_NO_CONNECTORS_MESSAGE`] fallback).
    pub connectors_info: &'a str,
    /// Detected current-repo HTTPS URL, if any.
    pub git_repo_url: Option<&'a str>,
    /// Pre-rendered environments list (IDs + names).
    pub environments_info: &'a str,
    /// Caption for a just-created default environment; empty to
    /// skip the note entirely.
    pub created_environment_note: &'a str,
    /// Pre-formatted setup-notes heads-up block (output of
    /// [`format_setup_notes`]). Empty when no setup issues.
    pub setup_notes: &'a str,
    /// `true` when the user's request likely needs GitHub repo
    /// access and the reminder line should be appended.
    pub needs_github_access_reminder: bool,
    /// User's skill invocation arguments. Empty ⇒ skip the
    /// initial AskUserQuestion reference + `## User Request`
    /// section; non-empty ⇒ skip the question and include the
    /// setup notes in the body instead.
    pub user_args: &'a str,
    /// When `true`, render the feature-flagged `/web-setup`
    /// hint; `false` ⇒ the GitHub-App reminder. Matches TS
    /// `getFeatureValue_CACHED_MAY_BE_STALE('tengu_cobalt_lantern', false)`.
    pub cobalt_lantern_enabled: bool,
}

/// Build the first-step paragraph. Depends on whether user args
/// were supplied. Port of TS
/// `firstStep` / `initialQuestion` in scheduleRemoteAgents.ts:162-172.
pub fn build_first_step(setup_notes: &str, user_args: &str) -> String {
    if !user_args.is_empty() {
        return "The user has already told you what they want (see User Request at the bottom). Skip the initial question and go directly to the matching workflow.".to_string();
    }
    let initial_question = if setup_notes.is_empty() {
        SCHEDULE_BASE_QUESTION.to_string()
    } else {
        format!("{setup_notes}\n\n{SCHEDULE_BASE_QUESTION}")
    };
    // The question is JSON-stringified in TS so the skill's
    // AskUserQuestion dialog stores it exactly. `serde_json`
    // produces the same escaping rules (\n, quotes).
    let json_question = serde_json::to_string(&initial_question)
        .unwrap_or_else(|_| format!("\"{initial_question}\""));
    format!(
        "Your FIRST action must be a single AskUserQuestion tool call (no preamble). Use this EXACT string for the `question` field — do not paraphrase or shorten it:\n\n{json_question}\n\nSet `header: \"Action\"` and offer the four actions (create/list/update/run) as options. After the user picks, follow the matching workflow below."
    )
}

/// Fill every slot and return the rendered prompt. Port of TS
/// `buildPrompt(opts)` at scheduleRemoteAgents.ts:135-322.
pub fn schedule_remote_agents_prompt(inputs: &ScheduleRemoteAgentsInputs<'_>) -> String {
    let first_step = build_first_step(inputs.setup_notes, inputs.user_args);
    let setup_notes_section =
        if !inputs.user_args.is_empty() && !inputs.setup_notes.is_empty() {
            format!("\n## Setup Notes\n\n{}\n", inputs.setup_notes)
        } else {
            String::new()
        };
    let git_repo_or_placeholder = inputs
        .git_repo_url
        .unwrap_or("https://github.com/ORG/REPO");
    let git_repo_workflow_hint = match inputs.git_repo_url {
        Some(url) => format!(
            " The default git repo is already set to `{url}`. Ask the user if this is the right repo or if they need a different one."
        ),
        None => " Ask which git repos the remote agent needs cloned into its environment.".to_string(),
    };
    let github_access_reminder = if inputs.needs_github_access_reminder {
        let inner = if inputs.cobalt_lantern_enabled {
            "they should run /web-setup to connect their GitHub account (or install the Claude GitHub App on the repo as an alternative) — otherwise the remote agent won't be able to access it"
        } else {
            "they need the Claude GitHub App installed on the repo — otherwise the remote agent won't be able to access it"
        };
        format!(
            "\n- If the user's request seems to require GitHub repo access (e.g. cloning a repo, opening PRs, reading code), remind them that {inner}."
        )
    } else {
        String::new()
    };
    let user_request_section = if !inputs.user_args.is_empty() {
        format!(
            "\n## User Request\n\nThe user said: \"{args}\"\n\nStart by understanding their intent and working through the appropriate workflow above.",
            args = inputs.user_args
        )
    } else {
        String::new()
    };

    SCHEDULE_PROMPT_TEMPLATE
        .replace("{{FIRST_STEP}}", &first_step)
        .replace("{{SETUP_NOTES_SECTION}}", &setup_notes_section)
        .replace("{{CONNECTORS_INFO}}", inputs.connectors_info)
        .replace("{{ENVIRONMENTS_INFO}}", inputs.environments_info)
        .replace(
            "{{CREATED_ENVIRONMENT_NOTE}}",
            inputs.created_environment_note,
        )
        .replace("{{USER_TIMEZONE}}", inputs.user_timezone)
        .replace("{{GIT_REPO_URL_OR_PLACEHOLDER}}", git_repo_or_placeholder)
        .replace("{{GIT_REPO_WORKFLOW_HINT}}", &git_repo_workflow_hint)
        .replace("{{GITHUB_ACCESS_REMINDER}}", &github_access_reminder)
        .replace("{{USER_REQUEST_SECTION}}", &user_request_section)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_inputs<'a>() -> ScheduleRemoteAgentsInputs<'a> {
        ScheduleRemoteAgentsInputs {
            user_timezone: "America/Los_Angeles",
            connectors_info: SCHEDULE_NO_CONNECTORS_MESSAGE,
            git_repo_url: None,
            environments_info: "- id: env_1, name: default",
            created_environment_note: "",
            setup_notes: "",
            needs_github_access_reminder: false,
            user_args: "",
            cobalt_lantern_enabled: false,
        }
    }

    #[test]
    fn sanitize_strips_claude_ai_prefix() {
        assert_eq!(sanitize_connector_name("Claude.ai-Slack"), "Slack");
        assert_eq!(sanitize_connector_name("claude ai Datadog"), "Datadog");
        assert_eq!(sanitize_connector_name("Claude-AI-Gmail"), "Gmail");
    }

    #[test]
    fn sanitize_replaces_disallowed_chars() {
        assert_eq!(sanitize_connector_name("My Service!"), "My-Service");
        assert_eq!(sanitize_connector_name("a.b.c"), "a-b-c");
        assert_eq!(
            sanitize_connector_name("Claude.ai-My Service.v2"),
            "My-Service-v2"
        );
    }

    #[test]
    fn sanitize_collapses_consecutive_dashes() {
        assert_eq!(sanitize_connector_name("a!!b"), "a-b");
    }

    #[test]
    fn format_setup_notes_bullets_lines() {
        let s = format_setup_notes(&["missing X", "need Y"]);
        assert!(s.starts_with("⚠ Heads-up:"));
        assert!(s.contains("- missing X"));
        assert!(s.contains("- need Y"));
    }

    #[test]
    fn first_step_without_args_includes_base_question() {
        let s = build_first_step("", "");
        assert!(s.contains("FIRST action must be a single AskUserQuestion"));
        // JSON-encoded question.
        assert!(s.contains("\"What would you like to do with scheduled remote agents?\""));
    }

    #[test]
    fn first_step_with_setup_notes_prepends_heads_up() {
        let notes = format_setup_notes(&["install GH App"]);
        let s = build_first_step(&notes, "");
        // The JSON-encoded question now contains the heads-up lines.
        assert!(s.contains("Heads-up"));
        assert!(s.contains("install GH App"));
    }

    #[test]
    fn first_step_with_user_args_skips_question() {
        let s = build_first_step("", "create a daily cron");
        assert!(s.starts_with("The user has already told you"));
        assert!(!s.contains("AskUserQuestion"));
    }

    #[test]
    fn prompt_has_canonical_headers() {
        let p = schedule_remote_agents_prompt(&default_inputs());
        assert!(p.starts_with("# Schedule Remote Agents"));
        assert!(p.contains("## First Step"));
        assert!(p.contains("## What You Can Do"));
        assert!(p.contains("## Create body shape"));
        assert!(p.contains("## Available MCP Connectors"));
        assert!(p.contains("## Environments"));
        assert!(p.contains("## API Field Reference"));
        assert!(p.contains("## Workflow"));
        assert!(p.contains("## Important Notes"));
    }

    #[test]
    fn prompt_fills_every_slot() {
        let p = schedule_remote_agents_prompt(&default_inputs());
        for slot in [
            "{{FIRST_STEP}}",
            "{{SETUP_NOTES_SECTION}}",
            "{{CONNECTORS_INFO}}",
            "{{ENVIRONMENTS_INFO}}",
            "{{CREATED_ENVIRONMENT_NOTE}}",
            "{{USER_TIMEZONE}}",
            "{{GIT_REPO_URL_OR_PLACEHOLDER}}",
            "{{GIT_REPO_WORKFLOW_HINT}}",
            "{{GITHUB_ACCESS_REMINDER}}",
            "{{USER_REQUEST_SECTION}}",
        ] {
            assert!(!p.contains(slot), "slot `{slot}` was not substituted");
        }
    }

    #[test]
    fn prompt_substitutes_timezone_in_cron_section() {
        let p = schedule_remote_agents_prompt(&default_inputs());
        // Timezone appears in "Cron Expression Examples" intro and
        // in the workflow's schedule step.
        assert!(p.contains("**America/Los_Angeles**"));
        assert!(p.contains("9am America/Los_Angeles = Xam UTC"));
    }

    #[test]
    fn prompt_without_git_repo_asks_user() {
        let p = schedule_remote_agents_prompt(&default_inputs());
        assert!(p.contains("Ask which git repos the remote agent needs cloned"));
        assert!(p.contains("https://github.com/ORG/REPO"));
    }

    #[test]
    fn prompt_with_git_repo_surfaces_url() {
        let mut i = default_inputs();
        let url = "https://github.com/acme/widgets";
        i.git_repo_url = Some(url);
        let p = schedule_remote_agents_prompt(&i);
        assert!(p.contains("https://github.com/acme/widgets"));
        assert!(p.contains("The default git repo is already set to"));
    }

    #[test]
    fn prompt_github_access_reminder_opts_in() {
        let mut i = default_inputs();
        // Off by default.
        assert!(!schedule_remote_agents_prompt(&i).contains("GitHub repo access"));
        i.needs_github_access_reminder = true;
        let p = schedule_remote_agents_prompt(&i);
        assert!(p.contains("GitHub repo access"));
        // cobalt_lantern off → GitHub App message.
        assert!(p.contains("Claude GitHub App installed on the repo"));
    }

    #[test]
    fn prompt_cobalt_lantern_uses_web_setup_phrasing() {
        let mut i = default_inputs();
        i.needs_github_access_reminder = true;
        i.cobalt_lantern_enabled = true;
        let p = schedule_remote_agents_prompt(&i);
        assert!(p.contains("/web-setup"));
    }

    #[test]
    fn prompt_user_args_appends_user_request_and_setup_notes() {
        let mut i = default_inputs();
        i.user_args = "daily security scan";
        let notes = format_setup_notes(&["install GH App"]);
        i.setup_notes = &notes;
        let p = schedule_remote_agents_prompt(&i);
        assert!(p.contains("## User Request"));
        assert!(p.contains("daily security scan"));
        // With args + setup notes, the body's Setup Notes section
        // is included so the notes aren't dropped.
        assert!(p.contains("## Setup Notes"));
        assert!(p.contains("install GH App"));
    }

    #[test]
    fn prompt_no_args_no_user_request_section() {
        let p = schedule_remote_agents_prompt(&default_inputs());
        assert!(!p.contains("## User Request"));
    }

    #[test]
    fn prompt_references_remote_trigger_tool_name() {
        let p = schedule_remote_agents_prompt(&default_inputs());
        assert!(p.contains("`RemoteTrigger`"));
        assert!(p.contains("ToolSearch select:RemoteTrigger"));
    }
}
