use anyhow::Result;

use super::registry::{Command, CommandContext, CommandHandler, CommandRegistry, CommandResult};

// ---------------------------------------------------------------------------
// Action-type command handlers
// ---------------------------------------------------------------------------

pub struct HelpHandler;
impl CommandHandler for HelpHandler {
    fn execute(&self, _args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        Ok(CommandResult::Action(
            "Available slash commands:\n\
             /help          - Show available commands\n\
             /status        - Show session status (model, tokens, messages)\n\
             /clear         - Clear conversation history\n\
             /compact       - Manually trigger conversation compaction\n\
             /model         - Show or change the current model\n\
             /config        - Show configuration\n\
             /cost          - Show token usage and estimated cost\n\
             /permissions   - Show current permission mode\n\
             /verbose       - Toggle verbose mode\n\
             /plan          - Enter plan mode\n\
             /exit-plan     - Exit plan mode\n\
             /commit        - Generate a git commit for staged changes\n\
             /review        - Review code changes (git diff)\n\
             /branch        - Create a new git branch\n\
             /pr            - Create a pull request description\n\
             /bug           - Report or analyze a bug\n\
             /test          - Generate tests for code\n\
             /refactor      - Suggest refactoring\n\
             /explain       - Explain code\n\
             /docs          - Generate documentation\n\
             /memory        - Show auto-memory files\n\
             /tasks         - Show current task list\n\
             /resume        - Resume a previous session\n\
             /fork          - Fork the current session\n\
             /context       - Show context window usage\n\
             /theme         - Toggle light/dark theme\n\
             /fast          - Toggle fast mode\n\
             /brief         - Toggle brief mode\n\
             /effort        - Set effort level"
                .to_string(),
        ))
    }
}

pub struct StatusHandler;
impl CommandHandler for StatusHandler {
    fn execute(&self, _args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        Ok(CommandResult::Action(format!(
            "Session status:\n  Model: {}\n  Working directory: {}",
            ctx.model,
            ctx.working_directory.display()
        )))
    }
}

pub struct ClearHandler;
impl CommandHandler for ClearHandler {
    fn execute(&self, _args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        Ok(CommandResult::Action(
            "Conversation history cleared.".to_string(),
        ))
    }
}

pub struct ModelHandler;
impl CommandHandler for ModelHandler {
    fn execute(&self, args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        if args.trim().is_empty() {
            Ok(CommandResult::Action(format!(
                "Current model: {}",
                ctx.model
            )))
        } else {
            Ok(CommandResult::Action(format!(
                "Model changed to: {}",
                args.trim()
            )))
        }
    }
}

pub struct ConfigHandler;
impl CommandHandler for ConfigHandler {
    fn execute(&self, _args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        Ok(CommandResult::Action(format!(
            "Configuration:\n  Model: {}\n  Working directory: {}",
            ctx.model,
            ctx.working_directory.display()
        )))
    }
}

pub struct CostHandler;
impl CommandHandler for CostHandler {
    fn execute(&self, _args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        Ok(CommandResult::Action(
            "Token usage:\n  Input tokens:  0\n  Output tokens: 0\n  Estimated cost: $0.00"
                .to_string(),
        ))
    }
}

pub struct PermissionsHandler;
impl CommandHandler for PermissionsHandler {
    fn execute(&self, _args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        Ok(CommandResult::Action(
            "Permission mode: default (ask before executing tools)".to_string(),
        ))
    }
}

pub struct VerboseHandler;
impl CommandHandler for VerboseHandler {
    fn execute(&self, _args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        Ok(CommandResult::Action("Verbose mode toggled.".to_string()))
    }
}

pub struct MemoryHandler;
impl CommandHandler for MemoryHandler {
    fn execute(&self, _args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        Ok(CommandResult::Action(
            "Auto-memory files:\n  ~/.claude/CLAUDE.md (global)\n  ./CLAUDE.md (project)"
                .to_string(),
        ))
    }
}

pub struct TasksHandler;
impl CommandHandler for TasksHandler {
    fn execute(&self, _args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        Ok(CommandResult::Action(
            "No active tasks.".to_string(),
        ))
    }
}

pub struct ResumeHandler;
impl CommandHandler for ResumeHandler {
    fn execute(&self, _args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        Ok(CommandResult::Action(
            "No previous sessions found to resume.".to_string(),
        ))
    }
}

pub struct ForkHandler;
impl CommandHandler for ForkHandler {
    fn execute(&self, _args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        Ok(CommandResult::Action(
            "Session forked. A new independent session has been created from this point."
                .to_string(),
        ))
    }
}

pub struct ContextHandler;
impl CommandHandler for ContextHandler {
    fn execute(&self, _args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        Ok(CommandResult::Action(
            "Context window usage:\n  Used:      0 tokens\n  Available: 200000 tokens\n  Utilization: 0%"
                .to_string(),
        ))
    }
}

pub struct ThemeHandler;
impl CommandHandler for ThemeHandler {
    fn execute(&self, _args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        Ok(CommandResult::Action("Theme toggled.".to_string()))
    }
}

pub struct FastHandler;
impl CommandHandler for FastHandler {
    fn execute(&self, _args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        Ok(CommandResult::Action("Fast mode toggled.".to_string()))
    }
}

pub struct BriefHandler;
impl CommandHandler for BriefHandler {
    fn execute(&self, _args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        Ok(CommandResult::Action("Brief mode toggled.".to_string()))
    }
}

pub struct EffortHandler;
impl CommandHandler for EffortHandler {
    fn execute(&self, args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        let level = args.trim();
        if level.is_empty() {
            Ok(CommandResult::Action(
                "Usage: /effort <low|medium|high>. Current effort level: medium".to_string(),
            ))
        } else {
            Ok(CommandResult::Action(format!(
                "Effort level set to: {}",
                level
            )))
        }
    }
}

// ---------------------------------------------------------------------------
// Prompt-type command handlers
// ---------------------------------------------------------------------------

pub struct CompactHandler;
impl CommandHandler for CompactHandler {
    fn execute(&self, _args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        Ok(CommandResult::Message(
            "Please summarize our conversation so far into a concise context that preserves \
             all important information, decisions, and code produced. Then continue from that \
             summary as the new conversation state."
                .to_string(),
        ))
    }
}

pub struct PlanHandler;
impl CommandHandler for PlanHandler {
    fn execute(&self, args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        let task = args.trim();
        if task.is_empty() {
            Ok(CommandResult::Message(
                "Enter plan mode: think carefully and produce a detailed, step-by-step plan \
                 for the task at hand. Do not execute any steps yet — only produce the plan \
                 and wait for approval."
                    .to_string(),
            ))
        } else {
            Ok(CommandResult::Message(format!(
                "Enter plan mode: think carefully and produce a detailed, step-by-step plan \
                 for the following task. Do not execute any steps yet — only produce the plan \
                 and wait for approval.\n\nTask: {}",
                task
            )))
        }
    }
}

pub struct ExitPlanHandler;
impl CommandHandler for ExitPlanHandler {
    fn execute(&self, _args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        Ok(CommandResult::Message(
            "Exit plan mode and proceed with executing the approved plan. \
             Begin with the first step."
                .to_string(),
        ))
    }
}

pub struct CommitHandler;
impl CommandHandler for CommitHandler {
    fn execute(&self, _args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        Ok(CommandResult::Message(
            "Generate a git commit message for the currently staged changes. \
             Run `git diff --cached` first to see what's staged."
                .to_string(),
        ))
    }
}

pub struct ReviewHandler;
impl CommandHandler for ReviewHandler {
    fn execute(&self, args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        let target = args.trim();
        if target.is_empty() {
            Ok(CommandResult::Message(
                "Review the current code changes by running `git diff`. Provide a thorough \
                 code review covering correctness, style, potential bugs, and suggestions \
                 for improvement."
                    .to_string(),
            ))
        } else {
            Ok(CommandResult::Message(format!(
                "Review the following code changes or file, providing a thorough code review \
                 covering correctness, style, potential bugs, and suggestions for improvement: {}",
                target
            )))
        }
    }
}

pub struct BranchHandler;
impl CommandHandler for BranchHandler {
    fn execute(&self, args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        let description = args.trim();
        if description.is_empty() {
            Ok(CommandResult::Message(
                "Create a new git branch. Suggest a suitable branch name based on the current \
                 context and recent work, then run `git checkout -b <branch-name>`."
                    .to_string(),
            ))
        } else {
            Ok(CommandResult::Message(format!(
                "Create a new git branch for the following work: \"{}\". \
                 Suggest a suitable branch name and run `git checkout -b <branch-name>`.",
                description
            )))
        }
    }
}

pub struct PrHandler;
impl CommandHandler for PrHandler {
    fn execute(&self, args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        let description = args.trim();
        if description.is_empty() {
            Ok(CommandResult::Message(
                "Create a pull request description for the current branch. Run `git log main..HEAD` \
                 and `git diff main` to understand the changes, then write a clear PR title, \
                 summary, and list of changes."
                    .to_string(),
            ))
        } else {
            Ok(CommandResult::Message(format!(
                "Create a pull request description for the following changes: \"{}\". \
                 Write a clear PR title, summary, and list of changes.",
                description
            )))
        }
    }
}

pub struct BugHandler;
impl CommandHandler for BugHandler {
    fn execute(&self, args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        let description = args.trim();
        if description.is_empty() {
            Ok(CommandResult::Message(
                "Analyze the current codebase for potential bugs. Look at recent changes \
                 with `git diff` and identify any issues with logic, error handling, \
                 edge cases, or regressions."
                    .to_string(),
            ))
        } else {
            Ok(CommandResult::Message(format!(
                "Analyze and help debug the following issue: \"{}\". \
                 Investigate the relevant code, identify the root cause, and suggest a fix.",
                description
            )))
        }
    }
}

pub struct TestHandler;
impl CommandHandler for TestHandler {
    fn execute(&self, args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        let target = args.trim();
        if target.is_empty() {
            Ok(CommandResult::Message(
                "Generate comprehensive tests for the current code. \
                 Identify untested functions or modules and write unit tests \
                 covering normal cases, edge cases, and error conditions."
                    .to_string(),
            ))
        } else {
            Ok(CommandResult::Message(format!(
                "Generate comprehensive tests for: \"{}\". \
                 Write unit tests covering normal cases, edge cases, and error conditions.",
                target
            )))
        }
    }
}

pub struct RefactorHandler;
impl CommandHandler for RefactorHandler {
    fn execute(&self, args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        let target = args.trim();
        if target.is_empty() {
            Ok(CommandResult::Message(
                "Suggest refactoring improvements for the current codebase. \
                 Look for code duplication, overly complex functions, poor naming, \
                 missing abstractions, and other code smells."
                    .to_string(),
            ))
        } else {
            Ok(CommandResult::Message(format!(
                "Suggest refactoring improvements for: \"{}\". \
                 Identify code smells, complexity issues, and opportunities to improve \
                 readability and maintainability.",
                target
            )))
        }
    }
}

pub struct ExplainHandler;
impl CommandHandler for ExplainHandler {
    fn execute(&self, args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        let target = args.trim();
        if target.is_empty() {
            Ok(CommandResult::Message(
                "Explain the current code. Provide a clear explanation of what it does, \
                 how it works, and the key design decisions."
                    .to_string(),
            ))
        } else {
            Ok(CommandResult::Message(format!(
                "Explain the following code or concept in detail: \"{}\". \
                 Describe what it does, how it works, and any important considerations.",
                target
            )))
        }
    }
}

pub struct DocsHandler;
impl CommandHandler for DocsHandler {
    fn execute(&self, args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        let target = args.trim();
        if target.is_empty() {
            Ok(CommandResult::Message(
                "Generate documentation for the current code. \
                 Write clear doc comments for all public functions, structs, and modules \
                 following the language's documentation conventions."
                    .to_string(),
            ))
        } else {
            Ok(CommandResult::Message(format!(
                "Generate documentation for: \"{}\". \
                 Write clear doc comments following the language's documentation conventions.",
                target
            )))
        }
    }
}

// ---------------------------------------------------------------------------
// Registry builder
// ---------------------------------------------------------------------------

pub fn build_default_commands() -> CommandRegistry {
    let mut registry = CommandRegistry::new();

    macro_rules! register {
        ($name:expr, $desc:expr, $typ:ident, $handler:expr) => {
            registry.register(Command {
                name: $name.to_string(),
                description: $desc.to_string(),
                command_type: CommandType::$typ,
                handler: Box::new($handler),
            });
        };
    }

    use super::registry::CommandType;

    // Action commands
    register!("help",        "Show available commands",                         Action, HelpHandler);
    register!("status",      "Show session status (model, tokens, messages)",   Action, StatusHandler);
    register!("clear",       "Clear conversation history",                      Action, ClearHandler);
    register!("model",       "Show or change the current model",                Action, ModelHandler);
    register!("config",      "Show configuration",                              Action, ConfigHandler);
    register!("cost",        "Show token usage and estimated cost",             Action, CostHandler);
    register!("permissions", "Show current permission mode",                    Action, PermissionsHandler);
    register!("verbose",     "Toggle verbose mode",                             Action, VerboseHandler);
    register!("memory",      "Show auto-memory files",                          Action, MemoryHandler);
    register!("tasks",       "Show current task list",                          Action, TasksHandler);
    register!("resume",      "Resume a previous session",                       Action, ResumeHandler);
    register!("fork",        "Fork the current session",                        Action, ForkHandler);
    register!("context",     "Show context window usage",                       Action, ContextHandler);
    register!("theme",       "Toggle light/dark theme",                         Action, ThemeHandler);
    register!("fast",        "Toggle fast mode",                                Action, FastHandler);
    register!("brief",       "Toggle brief mode",                               Action, BriefHandler);
    register!("effort",      "Set effort level",                                Action, EffortHandler);

    // Prompt commands
    register!("compact",   "Manually trigger conversation compaction",           Prompt, CompactHandler);
    register!("plan",      "Enter plan mode",                                    Prompt, PlanHandler);
    register!("exit-plan", "Exit plan mode",                                     Prompt, ExitPlanHandler);
    register!("commit",    "Generate a git commit for staged changes",           Prompt, CommitHandler);
    register!("review",    "Review code changes (git diff)",                     Prompt, ReviewHandler);
    register!("branch",    "Create a new git branch",                            Prompt, BranchHandler);
    register!("pr",        "Create a pull request description",                  Prompt, PrHandler);
    register!("bug",       "Report or analyze a bug",                            Prompt, BugHandler);
    register!("test",      "Generate tests for code",                            Prompt, TestHandler);
    register!("refactor",  "Suggest refactoring",                                Prompt, RefactorHandler);
    register!("explain",   "Explain code",                                       Prompt, ExplainHandler);
    register!("docs",      "Generate documentation",                             Prompt, DocsHandler);

    registry
}
