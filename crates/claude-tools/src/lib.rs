pub mod agent_tool;
pub mod agents;
pub mod ask_user;
pub mod bash;
pub mod bash_commands;
pub mod bash_sandbox_prompt;
pub mod bash_security;
pub mod brief_tool;
pub mod bundled_skills;
pub mod command_semantics;
pub mod config_tool;
pub mod cron_tool;
pub mod ctx_inspect_tool;
pub mod destructive_command_warning;
mod diff_utils;
pub mod edit;
pub mod edit_quote_style;
mod git_diff;
pub mod glob_tool;
pub mod grep;
pub mod list_peers_tool;
pub mod lsp_tool;
pub mod mcp_auth_tool;
pub mod mcp_resource_tools;
pub mod mcp_tool;
pub mod monitor_tool;
pub mod notebook_edit;
pub mod plan_mode;
pub mod powershell;
pub mod push_notification_tool;
pub mod read;
pub mod read_only_validation;
pub mod registry;
pub mod remote_trigger;
pub mod repl_tool;
pub mod sed_validation;
pub mod send_message;
pub mod send_user_file_tool;
pub mod skill_tool;
pub mod sleep_tool;
pub mod snip_tool;
pub mod subscribe_pr_tool;
pub mod suggest_background_pr_tool;
pub mod synthetic_output;
pub mod task_tools;
pub mod team_tools;
pub mod terminal_capture_tool;
pub mod todo_write;
mod tool_path;
mod tool_result_storage;
pub mod tool_search;
pub mod verify_plan_tool;
pub mod web_browser_tool;
pub mod web_fetch;
pub mod web_fetch_preapproved;
pub mod web_search;
pub mod workflow_tool;
pub mod worktree_tools;
pub mod write;

pub use mcp_resource_tools::{
    register_mcp_resource_tools, register_mcp_resource_tools_if_supported,
};
pub use mcp_tool::register_mcp_tools;
pub use registry::{ProgressSender, ReadFileState, ToolExecutor, ToolRegistry, ToolUseContext};
pub use tool_search::register_tool_search_snapshot;

use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default)]
pub struct RegistryOptions {
    pub is_non_interactive_session: bool,
}

/// Check whether an env-var-based feature flag is enabled.
/// Truthy values: `1`, `true`, `yes`, `on` (case-insensitive). Anything else
/// (including unset) is false. Mirrors the behaviour of TS `feature('X')`
/// + `isEnvTruthy(process.env.X)` in `tools.ts`.
fn feature_enabled(name: &str) -> bool {
    if claude_core::errors_util::is_env_definitely_falsy(name) {
        return false;
    }
    if matches!(
        name,
        "AGENT_TRIGGERS"
            | "AGENT_TRIGGERS_REMOTE"
            | "ENABLE_LSP_TOOL"
            | "KAIROS_PUSH_NOTIFICATION"
            | "MONITOR_TOOL"
            | "PROACTIVE"
    ) {
        return true;
    }
    claude_core::errors_util::is_env_truthy(name)
}

/// Internal/Anthropic user build, matching TS `process.env.USER_TYPE === 'ant'`.
/// Thin wrapper over the shared reader so the registry bootstrap stays
/// single-call-site.
fn is_ant_user() -> bool {
    claude_core::user_type::is_ant()
}

fn is_todo_v2_enabled(options: RegistryOptions) -> bool {
    feature_enabled("CLAUDE_CODE_ENABLE_TASKS") || !options.is_non_interactive_session
}

fn is_agent_swarms_enabled() -> bool {
    is_ant_user() || feature_enabled("CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS")
}

fn is_repl_mode_enabled() -> bool {
    if claude_core::errors_util::is_env_definitely_falsy("CLAUDE_CODE_REPL") {
        return false;
    }
    if feature_enabled("CLAUDE_REPL_MODE") {
        return true;
    }
    is_ant_user()
        && std::env::var("CLAUDE_CODE_ENTRYPOINT")
            .map(|entrypoint| entrypoint == "cli")
            .unwrap_or(false)
}

fn is_coordinator_mode_enabled() -> bool {
    feature_enabled("COORDINATOR_MODE") && claude_core::teams::is_coordinator_mode()
}

pub fn build_default_registry() -> ToolRegistry {
    build_default_registry_with_options(RegistryOptions::default())
}

pub fn build_default_registry_with_options(options: RegistryOptions) -> ToolRegistry {
    let mut reg = ToolRegistry::new();
    let todo_v2_enabled = is_todo_v2_enabled(options);

    if feature_enabled("CLAUDE_CODE_SIMPLE") {
        if is_repl_mode_enabled() && is_ant_user() {
            reg.register(Arc::new(repl_tool::REPLTool));
            if is_coordinator_mode_enabled() {
                reg.register(Arc::new(task_tools::TaskStopTool));
                reg.register(Arc::new(send_message::SendMessageTool));
            }
            return reg;
        }

        reg.register(Arc::new(bash::BashTool::new()));
        reg.register(Arc::new(read::FileReadTool));
        reg.register(Arc::new(edit::FileEditTool));
        if is_coordinator_mode_enabled() {
            reg.register(Arc::new(agent_tool::AgentTool));
            reg.register(Arc::new(task_tools::TaskStopTool));
            reg.register(Arc::new(send_message::SendMessageTool));
        }
        return reg;
    }

    // Baseline tools from TS tools.ts `getAllBaseTools()`, before feature-gated
    // spread entries. Tools with later feature gates must not appear here.
    reg.register(Arc::new(agent_tool::AgentTool));
    reg.register(Arc::new(task_tools::TaskOutputTool));
    reg.register(Arc::new(bash::BashTool::new()));
    if !claude_core::embedded_tools::has_embedded_search_tools() {
        reg.register(Arc::new(glob_tool::GlobTool));
        reg.register(Arc::new(grep::GrepTool));
    }
    reg.register(Arc::new(plan_mode::ExitPlanModeTool));
    reg.register(Arc::new(read::FileReadTool));
    reg.register(Arc::new(edit::FileEditTool));
    reg.register(Arc::new(write::FileWriteTool));
    reg.register(Arc::new(notebook_edit::NotebookEditTool));
    reg.register(Arc::new(web_fetch::WebFetchTool));
    reg.register(Arc::new(web_search::WebSearchTool));
    reg.register(Arc::new(task_tools::TaskStopTool));
    reg.register(Arc::new(ask_user::AskUserQuestionTool));
    reg.register(Arc::new(skill_tool::SkillTool));
    reg.register(Arc::new(plan_mode::EnterPlanModeTool));

    if todo_v2_enabled {
        reg.register(Arc::new(task_tools::TaskCreateTool));
        reg.register(Arc::new(task_tools::TaskGetTool));
        reg.register(Arc::new(task_tools::TaskUpdateTool));
        reg.register(Arc::new(task_tools::TaskListTool));
    } else {
        reg.register(Arc::new(todo_write::TodoWriteTool));
    }
    reg.register(Arc::new(worktree_tools::EnterWorktreeTool));
    reg.register(Arc::new(worktree_tools::ExitWorktreeTool));
    if is_agent_swarms_enabled() {
        reg.register(Arc::new(send_message::SendMessageTool));
    }

    if feature_enabled("ENABLE_LSP_TOOL") {
        reg.register(Arc::new(lsp_tool::LSPTool));
    }
    if is_agent_swarms_enabled() {
        reg.register(Arc::new(team_tools::TeamCreateTool));
        reg.register(Arc::new(team_tools::TeamDeleteTool));
    }
    if feature_enabled("PROACTIVE") || feature_enabled("KAIROS") {
        reg.register(Arc::new(sleep_tool::SleepTool));
    }
    if claude_core::shell_tool_utils::is_powershell_tool_enabled() {
        reg.register(Arc::new(powershell::PowerShellTool));
    }

    // ── USER_TYPE=ant gated (TS: process.env.USER_TYPE === 'ant') ────────────
    if is_ant_user() {
        reg.register(Arc::new(config_tool::ConfigTool::default()));
        reg.register(Arc::new(repl_tool::REPLTool));
        reg.register(Arc::new(
            suggest_background_pr_tool::SuggestBackgroundPRTool,
        ));
    }

    // ── Cron / agent-trigger gated. See cron_tool::is_kairos_cron_enabled
    // for the combined AGENT_TRIGGERS + CLAUDE_CODE_DISABLE_CRON gate
    // (matches TS `isKairosCronEnabled`).
    if cron_tool::is_kairos_cron_enabled() {
        reg.register(Arc::new(cron_tool::ScheduleCronTool));
        reg.register(Arc::new(cron_tool::CronDeleteTool));
        reg.register(Arc::new(cron_tool::CronListTool));
    }
    if feature_enabled("AGENT_TRIGGERS_REMOTE") {
        reg.register(Arc::new(remote_trigger::RemoteTriggerTool));
    }

    // ── Kairos / desktop-bridge gated (TS: feature('KAIROS*')) ───────────────
    if feature_enabled("KAIROS") || feature_enabled("KAIROS_PUSH_NOTIFICATION") {
        reg.register(Arc::new(push_notification_tool::PushNotificationTool));
    }
    if feature_enabled("KAIROS") {
        reg.register(Arc::new(send_user_file_tool::SendUserFileTool));
    }
    if feature_enabled("KAIROS_GITHUB_WEBHOOKS") {
        reg.register(Arc::new(subscribe_pr_tool::SubscribePRTool));
    }

    // ── Experimental / internal gated (one env var each) ─────────────────────
    if feature_enabled("MONITOR_TOOL") {
        reg.register(Arc::new(monitor_tool::MonitorTool));
    }
    if feature_enabled("CONTEXT_COLLAPSE") {
        reg.register(Arc::new(ctx_inspect_tool::CtxInspectTool));
    }
    if feature_enabled("TERMINAL_PANEL") {
        reg.register(Arc::new(terminal_capture_tool::TerminalCaptureTool));
    }
    if feature_enabled("HISTORY_SNIP") {
        reg.register(Arc::new(snip_tool::SnipTool));
    }
    if feature_enabled("WEB_BROWSER_TOOL") {
        reg.register(Arc::new(web_browser_tool::WebBrowserTool));
    }
    if feature_enabled("UDS_INBOX") {
        reg.register(Arc::new(list_peers_tool::ListPeersTool));
    }
    if feature_enabled("WORKFLOW_SCRIPTS") {
        reg.register(Arc::new(workflow_tool::WorkflowTool));
    }
    if feature_enabled("CLAUDE_CODE_VERIFY_PLAN") {
        reg.register(Arc::new(verify_plan_tool::VerifyPlanExecutionTool));
    }

    if is_repl_mode_enabled() && reg.get("REPL").is_some() {
        for name in [
            "Read",
            "Write",
            "Edit",
            "Glob",
            "Grep",
            "Bash",
            "NotebookEdit",
            "Agent",
        ] {
            reg.remove(name);
        }
    }

    // TS includes ToolSearch optimistically after assembling the base tool set.
    // The request-time API layer later decides whether any tools are deferred.
    tool_search::register_tool_search_snapshot(&mut reg);

    reg
}

pub fn filter_registry_by_deny_rules(
    registry: &mut ToolRegistry,
    deny_rules: &[claude_core::config::settings::PermissionRuleConfig],
) {
    let denied_names = registry
        .all()
        .iter()
        .filter(|tool| {
            deny_rules
                .iter()
                .any(|rule| rule_denies_whole_tool(tool.name(), rule))
        })
        .map(|tool| tool.name().to_string())
        .collect::<Vec<_>>();

    for name in denied_names {
        registry.remove(&name);
    }
}

fn rule_denies_whole_tool(
    tool_name: &str,
    rule: &claude_core::config::settings::PermissionRuleConfig,
) -> bool {
    if rule
        .pattern
        .as_deref()
        .is_some_and(|pattern| !pattern.is_empty() && pattern != "*")
    {
        return false;
    }

    if rule.tool == tool_name {
        return true;
    }

    let Some((rule_server, rule_tool)) = parse_mcp_tool_name(&rule.tool) else {
        return false;
    };
    let Some((tool_server, _)) = parse_mcp_tool_name(tool_name) else {
        return false;
    };

    rule_server == tool_server && rule_tool.is_none_or(|name| name == "*")
}

fn parse_mcp_tool_name(name: &str) -> Option<(&str, Option<&str>)> {
    let rest = name.strip_prefix("mcp__")?;
    match rest.split_once("__") {
        Some((server, tool)) => Some((server, Some(tool))),
        None => Some((rest, None)),
    }
}
