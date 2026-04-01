pub mod registry;
pub mod bash;
pub mod read;
pub mod write;
pub mod edit;
pub mod grep;
pub mod glob_tool;
pub mod lsp_tool;

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
    reg.register(Arc::new(lsp_tool::LSPTool));
    reg
}
