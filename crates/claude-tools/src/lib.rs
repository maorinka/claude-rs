pub mod registry;
pub mod bash;
pub mod bash_security;
pub mod read;
pub mod write;
pub mod edit;
pub mod grep;
pub mod glob_tool;
pub mod web_fetch;
pub mod web_search;
pub mod task_tools;
pub mod notebook_edit;
pub mod agent_tool;
pub mod agents;
pub mod mcp_tool;
pub mod config_tool;
pub mod plan_mode;
pub mod ask_user;
pub mod brief_tool;
pub mod send_message;
pub mod lsp_tool;
pub mod tool_search;
pub mod team_tools;
pub mod remote_trigger;
pub mod worktree_tools;
pub mod mcp_resource_tools;
pub mod powershell;
pub mod cron_tool;
pub mod skill_tool;
pub mod sleep_tool;
pub mod synthetic_output;
pub mod todo_write;
pub mod mcp_auth_tool;
pub mod repl_tool;

pub use registry::{ToolExecutor, ToolRegistry, ToolUseContext, ProgressSender, ReadFileState};
pub use mcp_tool::register_mcp_tools;

use std::sync::Arc;

pub fn build_default_registry() -> ToolRegistry {
    let mut reg = ToolRegistry::new();
    reg.register(Arc::new(bash::BashTool));
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
    reg.register(Arc::new(config_tool::ConfigTool::default()));
    reg.register(Arc::new(plan_mode::EnterPlanModeTool));
    reg.register(Arc::new(plan_mode::ExitPlanModeTool));
    reg.register(Arc::new(ask_user::AskUserQuestionTool));
    reg.register(Arc::new(brief_tool::BriefTool));
    reg.register(Arc::new(send_message::SendMessageTool));
    reg.register(Arc::new(lsp_tool::LSPTool));
    reg.register(Arc::new(tool_search::ToolSearchTool));
    reg.register(Arc::new(team_tools::TeamCreateTool));
    reg.register(Arc::new(team_tools::TeamDeleteTool));
    reg.register(Arc::new(remote_trigger::RemoteTriggerTool));
    reg.register(Arc::new(worktree_tools::EnterWorktreeTool));
    reg.register(Arc::new(worktree_tools::ExitWorktreeTool));
    reg.register(Arc::new(mcp_resource_tools::ListMcpResourcesTool));
    reg.register(Arc::new(mcp_resource_tools::ReadMcpResourceTool));
    reg.register(Arc::new(powershell::PowerShellTool));
    reg.register(Arc::new(cron_tool::ScheduleCronTool));
    reg.register(Arc::new(skill_tool::SkillTool));
    reg.register(Arc::new(sleep_tool::SleepTool));
    reg.register(Arc::new(synthetic_output::SyntheticOutputTool));
    reg.register(Arc::new(todo_write::TodoWriteTool));
    reg.register(Arc::new(mcp_auth_tool::McpAuthTool));
    reg.register(Arc::new(repl_tool::REPLTool));
    reg
}
