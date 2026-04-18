pub mod agent_tool;
pub mod agents;
pub mod ask_user;
pub mod bash;
pub mod bash_commands;
pub mod bash_sandbox_prompt;
pub mod bash_security;
pub mod brief_tool;
pub mod config_tool;
pub mod command_semantics;
pub mod cron_tool;
pub mod destructive_command_warning;
pub mod ctx_inspect_tool;
pub mod edit;
pub mod edit_quote_style;
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
pub mod sed_validation;
pub mod remote_trigger;
pub mod repl_tool;
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
pub mod tool_search;
pub mod verify_plan_tool;
pub mod web_browser_tool;
pub mod web_fetch;
pub mod web_fetch_preapproved;
pub mod web_search;
pub mod workflow_tool;
pub mod worktree_tools;
pub mod write;

pub use mcp_tool::register_mcp_tools;
pub use registry::{ProgressSender, ReadFileState, ToolExecutor, ToolRegistry, ToolUseContext};

use std::sync::Arc;

/// Check whether an env-var-based feature flag is enabled.
/// Truthy values: `1`, `true`, `yes`, `on` (case-insensitive). Anything else
/// (including unset) is false. Mirrors the behaviour of TS `feature('X')`
/// + `isEnvTruthy(process.env.X)` in `tools.ts`.
fn feature_enabled(name: &str) -> bool {
    claude_core::errors_util::is_env_truthy(name)
}

/// Internal/Anthropic user build, matching TS `process.env.USER_TYPE === 'ant'`.
/// Thin wrapper over the shared reader so the registry bootstrap stays
/// single-call-site.
fn is_ant_user() -> bool {
    claude_core::user_type::is_ant()
}

pub fn build_default_registry() -> ToolRegistry {
    let mut reg = ToolRegistry::new();

    // ── Baseline tools (always registered in TS tools.ts getAllBaseTools()) ──
    reg.register(Arc::new(bash::BashTool::new()));
    reg.register(Arc::new(read::FileReadTool));
    reg.register(Arc::new(write::FileWriteTool));
    reg.register(Arc::new(edit::FileEditTool));
    reg.register(Arc::new(grep::GrepTool));
    reg.register(Arc::new(glob_tool::GlobTool));
    reg.register(Arc::new(web_fetch::WebFetchTool));
    // WebSearchTool is a server-side tool (handled by the API, not client-side).
    // It is NOT registered here. Its definition is injected into the API request.
    reg.register(Arc::new(task_tools::TaskCreateTool));
    reg.register(Arc::new(task_tools::TaskListTool));
    reg.register(Arc::new(task_tools::TaskUpdateTool));
    reg.register(Arc::new(task_tools::TaskGetTool));
    reg.register(Arc::new(task_tools::TaskStopTool));
    reg.register(Arc::new(task_tools::TaskOutputTool));
    reg.register(Arc::new(notebook_edit::NotebookEditTool));
    reg.register(Arc::new(agent_tool::AgentTool));
    reg.register(Arc::new(plan_mode::EnterPlanModeTool));
    reg.register(Arc::new(plan_mode::ExitPlanModeTool));
    reg.register(Arc::new(ask_user::AskUserQuestionTool));
    reg.register(Arc::new(brief_tool::BriefTool));
    reg.register(Arc::new(send_message::SendMessageTool));
    reg.register(Arc::new(lsp_tool::LSPTool));
    reg.register(Arc::new(tool_search::ToolSearchTool));
    reg.register(Arc::new(team_tools::TeamCreateTool));
    reg.register(Arc::new(team_tools::TeamDeleteTool));
    reg.register(Arc::new(worktree_tools::EnterWorktreeTool));
    reg.register(Arc::new(worktree_tools::ExitWorktreeTool));
    reg.register(Arc::new(mcp_resource_tools::ListMcpResourcesTool));
    reg.register(Arc::new(mcp_resource_tools::ReadMcpResourceTool));
    reg.register(Arc::new(powershell::PowerShellTool));
    reg.register(Arc::new(skill_tool::SkillTool));
    reg.register(Arc::new(sleep_tool::SleepTool));
    reg.register(Arc::new(synthetic_output::SyntheticOutputTool));
    reg.register(Arc::new(todo_write::TodoWriteTool));
    reg.register(Arc::new(mcp_auth_tool::McpAuthTool));

    // ── USER_TYPE=ant gated (TS: process.env.USER_TYPE === 'ant') ────────────
    if is_ant_user() {
        reg.register(Arc::new(config_tool::ConfigTool::default()));
        reg.register(Arc::new(repl_tool::REPLTool));
        reg.register(Arc::new(
            suggest_background_pr_tool::SuggestBackgroundPRTool,
        ));
    }

    // ── Cron / agent-trigger gated (TS: feature('AGENT_TRIGGERS')) ───────────
    if feature_enabled("AGENT_TRIGGERS") {
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

    reg
}
