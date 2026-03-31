pub mod registry;
pub mod bash;
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

pub use registry::{ToolExecutor, ToolRegistry, ToolUseContext, ProgressSender};

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
    reg.register(Arc::new(web_search::WebSearchTool));
    reg.register(Arc::new(task_tools::TaskCreateTool));
    reg.register(Arc::new(task_tools::TaskListTool));
    reg.register(Arc::new(task_tools::TaskUpdateTool));
    reg.register(Arc::new(task_tools::TaskGetTool));
    reg.register(Arc::new(task_tools::TaskStopTool));
    reg.register(Arc::new(task_tools::TaskOutputTool));
    reg.register(Arc::new(notebook_edit::NotebookEditTool));
    reg.register(Arc::new(agent_tool::AgentTool));
    reg
}
