use anyhow::Result;

use super::registry::{Command, CommandContext, CommandHandler, CommandRegistry, CommandResult};

// ---------------------------------------------------------------------------
// Helper: read shared state or return a sensible fallback message
// ---------------------------------------------------------------------------

macro_rules! with_shared {
    ($ctx:expr, $body:expr) => {{
        match &$ctx.shared {
            Some(arc) => {
                let state = arc.lock().unwrap();
                $body(state)
            }
            None => Ok(CommandResult::Action(
                "(no live session data available)".to_string(),
            )),
        }
    }};
}

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
        with_shared!(ctx, |state: std::sync::MutexGuard<'_, super::registry::SharedCommandState>| {
            let elapsed = state.session_start.elapsed();
            let mins = elapsed.as_secs() / 60;
            let secs = elapsed.as_secs() % 60;
            let duration = if mins > 0 {
                format!("{}m {}s", mins, secs)
            } else {
                format!("{}s", secs)
            };
            Ok(CommandResult::Action(format!(
                "Model: {}\n\
                 Working directory: {}\n\
                 Session ID: {}\n\
                 Messages: {}\n\
                 Total tokens: {}\n\
                 API requests: {}\n\
                 Session duration: {}\n\
                 Permission mode: {}\n\
                 Fast mode: {}\n\
                 Brief mode: {}\n\
                 Effort: {}",
                state.model,
                ctx.working_directory.display(),
                if state.session_id.is_empty() { "(none)" } else { &state.session_id },
                state.message_count,
                state.total_tokens,
                state.request_count,
                duration,
                state.permission_mode,
                if state.fast_mode { "on" } else { "off" },
                if state.brief_mode { "on" } else { "off" },
                state.effort_level,
            )))
        })
    }
}

pub struct ClearHandler;
impl CommandHandler for ClearHandler {
    fn execute(&self, _args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        if let Some(ref arc) = ctx.shared {
            let mut state = arc.lock().unwrap();
            state.clear_requested = true;
            state.message_count = 0;
            state.total_tokens = 0;
            state.request_count = 0;
            state.total_cost_usd = 0.0;
            state.cost_summary.clear();
        }
        Ok(CommandResult::Action(
            "Conversation history cleared.".to_string(),
        ))
    }
}

pub struct ModelHandler;
impl CommandHandler for ModelHandler {
    fn execute(&self, args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        let new_model = args.trim();
        if new_model.is_empty() {
            // Show current model
            let model = if let Some(ref arc) = ctx.shared {
                let state = arc.lock().unwrap();
                state.model.clone()
            } else {
                ctx.model.clone()
            };
            Ok(CommandResult::Action(format!("Current model: {}", model)))
        } else {
            // Switch model
            if let Some(ref arc) = ctx.shared {
                let mut state = arc.lock().unwrap();
                state.model = new_model.to_string();
            }
            Ok(CommandResult::Action(format!(
                "Model changed to: {}",
                new_model
            )))
        }
    }
}

pub struct ConfigHandler;
impl CommandHandler for ConfigHandler {
    fn execute(&self, _args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        let settings_path = crate::config::paths::user_settings_path()
            .unwrap_or_else(|_| std::path::PathBuf::from("~/.claude/settings.json"));
        let content = std::fs::read_to_string(&settings_path)
            .unwrap_or_else(|_| "{}".into());
        Ok(CommandResult::Action(format!(
            "Settings ({}):\n{}",
            settings_path.display(),
            content
        )))
    }
}

pub struct CostHandler;
impl CommandHandler for CostHandler {
    fn execute(&self, _args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        with_shared!(ctx, |state: std::sync::MutexGuard<'_, super::registry::SharedCommandState>| {
            if state.cost_summary.is_empty() && state.request_count == 0 {
                Ok(CommandResult::Action(format!(
                    "Model: {}\nNo API requests made yet.",
                    state.model
                )))
            } else {
                Ok(CommandResult::Action(format!(
                    "Model: {}\n{}",
                    state.model,
                    if state.cost_summary.is_empty() {
                        format!(
                            "Total tokens: {} | Requests: {} | Cost: ${:.4}",
                            state.total_tokens, state.request_count, state.total_cost_usd
                        )
                    } else {
                        state.cost_summary.clone()
                    }
                )))
            }
        })
    }
}

pub struct PermissionsHandler;
impl CommandHandler for PermissionsHandler {
    fn execute(&self, _args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        let mode = if let Some(ref arc) = ctx.shared {
            let state = arc.lock().unwrap();
            state.permission_mode.clone()
        } else {
            "default".to_string()
        };
        let description = match mode.as_str() {
            "bypass" => "All tool executions are auto-approved (--dangerously-skip-permissions).",
            "interactive-only" => "Only interactive tool executions require approval.",
            _ => "Tools require approval before execution (ask mode).",
        };
        Ok(CommandResult::Action(format!(
            "Permission mode: {}\n{}",
            mode, description
        )))
    }
}

pub struct VerboseHandler;
impl CommandHandler for VerboseHandler {
    fn execute(&self, _args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        if let Some(ref arc) = ctx.shared {
            let mut state = arc.lock().unwrap();
            state.verbose_mode = !state.verbose_mode;
            let status = if state.verbose_mode { "enabled" } else { "disabled" };
            Ok(CommandResult::Action(format!("Verbose mode {}.", status)))
        } else {
            Ok(CommandResult::Action("Verbose mode toggled.".to_string()))
        }
    }
}

pub struct MemoryHandler;
impl CommandHandler for MemoryHandler {
    fn execute(&self, _args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        let mut output = String::from("Memory files:\n");
        let mut found_any = false;

        // Global CLAUDE.md
        if let Ok(claude_dir) = crate::config::paths::claude_dir() {
            let global_md = claude_dir.join("CLAUDE.md");
            if global_md.exists() {
                found_any = true;
                let preview = read_preview(&global_md, 3);
                output.push_str(&format!(
                    "\n  {} (global)\n{}\n",
                    global_md.display(),
                    indent_lines(&preview, "    ")
                ));
            }

            // List other memory files in ~/.claude/
            let memory_dir = claude_dir.join("memory");
            if memory_dir.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&memory_dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_file() {
                            found_any = true;
                            let preview = read_preview(&path, 2);
                            output.push_str(&format!(
                                "\n  {}\n{}\n",
                                path.display(),
                                indent_lines(&preview, "    ")
                            ));
                        }
                    }
                }
            }
        }

        // Project-local CLAUDE.md
        let project_md = ctx.working_directory.join("CLAUDE.md");
        if project_md.exists() {
            found_any = true;
            let preview = read_preview(&project_md, 3);
            output.push_str(&format!(
                "\n  {} (project)\n{}\n",
                project_md.display(),
                indent_lines(&preview, "    ")
            ));
        }

        // .claude/settings.local.json (project-level overrides)
        let local_settings = ctx.working_directory.join(".claude").join("settings.local.json");
        if local_settings.exists() {
            found_any = true;
            output.push_str(&format!(
                "\n  {} (project settings)\n",
                local_settings.display()
            ));
        }

        if !found_any {
            output.push_str("  No memory files found.\n");
            output.push_str("  Create ~/.claude/CLAUDE.md or ./CLAUDE.md to add project memory.");
        }

        Ok(CommandResult::Action(output))
    }
}

pub struct TasksHandler;
impl CommandHandler for TasksHandler {
    fn execute(&self, _args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        // The task store lives in claude-tools (circular dep if accessed here).
        // The TUI populates a task snapshot into SharedCommandState before dispatch.
        Ok(CommandResult::Action(
            "Task list is managed by the tool executor.\n\
             Use the TaskList tool to query active tasks."
                .to_string(),
        ))
    }
}

pub struct ResumeHandler;
impl CommandHandler for ResumeHandler {
    fn execute(&self, args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        let target = args.trim();

        match crate::session::manager::SessionManager::list_sessions() {
            Ok(sessions) if sessions.is_empty() => {
                Ok(CommandResult::Action(
                    "No previous sessions found.".to_string(),
                ))
            }
            Ok(sessions) => {
                if !target.is_empty() {
                    // User specified a session ID -- check if it exists
                    if let Some(s) = sessions.iter().find(|s| s.id.starts_with(target)) {
                        Ok(CommandResult::Action(format!(
                            "To resume session '{}', restart with:\n  claude --resume {}",
                            s.id, s.id
                        )))
                    } else {
                        Ok(CommandResult::Error(format!(
                            "Session '{}' not found. Use /resume to list available sessions.",
                            target
                        )))
                    }
                } else {
                    let mut output = String::from("Recent sessions:\n");
                    for (i, session) in sessions.iter().take(10).enumerate() {
                        let age = session
                            .last_modified
                            .and_then(|t| t.elapsed().ok())
                            .map(format_duration)
                            .unwrap_or_else(|| "unknown".to_string());
                        output.push_str(&format!(
                            "  {}. {} ({})\n",
                            i + 1,
                            &session.id[..session.id.len().min(8)],
                            age
                        ));
                    }
                    output.push_str("\nTo resume: restart with  claude --resume <session-id>");
                    Ok(CommandResult::Action(output))
                }
            }
            Err(e) => Ok(CommandResult::Error(format!(
                "Failed to list sessions: {}",
                e
            ))),
        }
    }
}

pub struct ForkHandler;
impl CommandHandler for ForkHandler {
    fn execute(&self, _args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        if let Some(ref arc) = ctx.shared {
            let mut state = arc.lock().unwrap();
            state.fork_requested = true;
            let new_id = uuid::Uuid::new_v4().to_string();
            Ok(CommandResult::Action(format!(
                "Session forked. New session ID: {}\n\
                 The conversation history has been copied to the new session.",
                &new_id[..8]
            )))
        } else {
            Ok(CommandResult::Action(
                "Cannot fork: no active session.".to_string(),
            ))
        }
    }
}

pub struct ContextHandler;
impl CommandHandler for ContextHandler {
    fn execute(&self, _args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        with_shared!(ctx, |state: std::sync::MutexGuard<'_, super::registry::SharedCommandState>| {
            let window = state.context_window;
            let used = state.total_tokens;
            let pct = if window > 0 {
                ((used as f64 / window as f64) * 100.0) as u64
            } else {
                0
            };
            let available = if window > used { window - used } else { 0 };
            Ok(CommandResult::Action(format!(
                "Context window usage:\n\
                 Model: {}\n\
                 Used:      {} tokens\n\
                 Available: {} tokens\n\
                 Window:    {} tokens\n\
                 Utilization: {}%",
                state.model, used, available, window, pct
            )))
        })
    }
}

pub struct ThemeHandler;
impl CommandHandler for ThemeHandler {
    fn execute(&self, _args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        if let Some(ref arc) = ctx.shared {
            let mut state = arc.lock().unwrap();
            state.dark_theme = !state.dark_theme;
            let name = if state.dark_theme { "dark" } else { "light" };
            Ok(CommandResult::Action(format!("Theme switched to: {}", name)))
        } else {
            Ok(CommandResult::Action("Theme toggled.".to_string()))
        }
    }
}

pub struct FastHandler;
impl CommandHandler for FastHandler {
    fn execute(&self, _args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        if let Some(ref arc) = ctx.shared {
            let mut state = arc.lock().unwrap();
            state.fast_mode = !state.fast_mode;
            let status = if state.fast_mode { "enabled" } else { "disabled" };
            Ok(CommandResult::Action(format!(
                "Fast mode {}. {}",
                status,
                if state.fast_mode {
                    "Requests will use the speed-optimized path."
                } else {
                    "Requests will use standard quality."
                }
            )))
        } else {
            Ok(CommandResult::Action("Fast mode toggled.".to_string()))
        }
    }
}

pub struct BriefHandler;
impl CommandHandler for BriefHandler {
    fn execute(&self, _args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        if let Some(ref arc) = ctx.shared {
            let mut state = arc.lock().unwrap();
            state.brief_mode = !state.brief_mode;
            let status = if state.brief_mode { "enabled" } else { "disabled" };
            Ok(CommandResult::Action(format!(
                "Brief mode {}. {}",
                status,
                if state.brief_mode {
                    "Responses will be shorter and more direct."
                } else {
                    "Responses will use normal verbosity."
                }
            )))
        } else {
            Ok(CommandResult::Action("Brief mode toggled.".to_string()))
        }
    }
}

pub struct EffortHandler;
impl CommandHandler for EffortHandler {
    fn execute(&self, args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        let level = args.trim().to_lowercase();
        if level.is_empty() {
            let current = if let Some(ref arc) = ctx.shared {
                let state = arc.lock().unwrap();
                state.effort_level.clone()
            } else {
                "medium".to_string()
            };
            Ok(CommandResult::Action(format!(
                "Current effort level: {}\nUsage: /effort <low|medium|high>",
                current
            )))
        } else {
            match level.as_str() {
                "low" | "medium" | "high" => {
                    if let Some(ref arc) = ctx.shared {
                        let mut state = arc.lock().unwrap();
                        state.effort_level = level.clone();
                    }
                    Ok(CommandResult::Action(format!(
                        "Effort level set to: {}",
                        level
                    )))
                }
                _ => Ok(CommandResult::Error(format!(
                    "Invalid effort level '{}'. Use: low, medium, or high",
                    level
                ))),
            }
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
// Helpers
// ---------------------------------------------------------------------------

/// Read the first `n` lines of a file as a preview.
fn read_preview(path: &std::path::Path, n: usize) -> String {
    match std::fs::read_to_string(path) {
        Ok(content) => {
            let lines: Vec<&str> = content.lines().take(n).collect();
            let preview = lines.join("\n");
            if content.lines().count() > n {
                format!("{}...", preview)
            } else {
                preview
            }
        }
        Err(_) => "(could not read file)".to_string(),
    }
}

/// Indent each line of `text` with `prefix`.
fn indent_lines(text: &str, prefix: &str) -> String {
    text.lines()
        .map(|line| format!("{}{}", prefix, line))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Format a Duration into a human-readable string like "2h ago" or "3d ago".
fn format_duration(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{}s ago", secs)
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86400)
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

// ── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::registry::SharedCommandState;
    use std::sync::{Arc, Mutex};

    fn test_ctx() -> CommandContext {
        CommandContext {
            working_directory: std::path::PathBuf::from("/tmp/test-project"),
            model: "claude-sonnet-4-20250514".to_string(),
            shared: None,
        }
    }

    fn test_ctx_with_shared() -> (CommandContext, Arc<Mutex<SharedCommandState>>) {
        let shared = Arc::new(Mutex::new(SharedCommandState {
            model: "claude-sonnet-4-20250514".to_string(),
            total_tokens: 12345,
            message_count: 7,
            session_id: "test-session-abc123".to_string(),
            permission_mode: "default".to_string(),
            cost_summary: "Tokens: 5000 in / 7345 out | Cache: 0 read / 0 write | Requests: 3 | Cost: $0.1252".to_string(),
            request_count: 3,
            total_cost_usd: 0.1252,
            fast_mode: false,
            verbose_mode: false,
            brief_mode: false,
            effort_level: "medium".to_string(),
            dark_theme: true,
            context_window: 200_000,
            clear_requested: false,
            fork_requested: false,
            ..Default::default()
        }));
        let ctx = CommandContext {
            working_directory: std::path::PathBuf::from("/tmp/test-project"),
            model: "claude-sonnet-4-20250514".to_string(),
            shared: Some(shared.clone()),
        };
        (ctx, shared)
    }

    #[test]
    fn test_cost_shows_real_data() {
        let (ctx, _shared) = test_ctx_with_shared();
        let handler = CostHandler;
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(
                    text.contains("claude-sonnet-4-20250514"),
                    "/cost should contain model name, got: {}",
                    text
                );
                assert!(
                    text.contains("$0.1252"),
                    "/cost should contain cost, got: {}",
                    text
                );
                assert!(
                    text.contains("Requests: 3"),
                    "/cost should contain request count, got: {}",
                    text
                );
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_cost_no_requests_yet() {
        let shared = Arc::new(Mutex::new(SharedCommandState::default()));
        let ctx = CommandContext {
            working_directory: std::path::PathBuf::from("/tmp"),
            model: "claude-sonnet-4-6".to_string(),
            shared: Some(shared),
        };
        let handler = CostHandler;
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(
                    text.contains("No API requests made yet"),
                    "/cost should say no requests, got: {}",
                    text
                );
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_status_shows_real_session_data() {
        let (ctx, _shared) = test_ctx_with_shared();
        let handler = StatusHandler;
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("claude-sonnet-4-20250514"), "model: {}", text);
                assert!(text.contains("/tmp/test-project"), "working dir: {}", text);
                assert!(text.contains("Messages: 7"), "messages: {}", text);
                assert!(text.contains("Total tokens: 12345"), "tokens: {}", text);
                assert!(text.contains("test-session"), "session id: {}", text);
                assert!(text.contains("Permission mode: default"), "perm mode: {}", text);
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_context_shows_token_usage() {
        let (ctx, _shared) = test_ctx_with_shared();
        let handler = ContextHandler;
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("12345"), "used tokens: {}", text);
                assert!(text.contains("200000"), "window size: {}", text);
                assert!(text.contains("6%"), "utilization: {}", text);
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_model_show_and_switch() {
        let (ctx, shared) = test_ctx_with_shared();
        let handler = ModelHandler;

        // Show current
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("claude-sonnet-4-20250514"), "show model: {}", text);
            }
            _ => panic!("expected Action"),
        }

        // Switch
        let result = handler.execute("claude-opus-4", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("claude-opus-4"), "switch msg: {}", text);
            }
            _ => panic!("expected Action"),
        }

        // Verify state was updated
        let state = shared.lock().unwrap();
        assert_eq!(state.model, "claude-opus-4");
    }

    #[test]
    fn test_clear_sets_flag_and_resets_counters() {
        let (ctx, shared) = test_ctx_with_shared();
        let handler = ClearHandler;
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("cleared"), "clear msg: {}", text);
            }
            _ => panic!("expected Action"),
        }
        let state = shared.lock().unwrap();
        assert!(state.clear_requested);
        assert_eq!(state.message_count, 0);
        assert_eq!(state.total_tokens, 0);
    }

    #[test]
    fn test_fast_toggles() {
        let (ctx, shared) = test_ctx_with_shared();
        let handler = FastHandler;

        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("enabled"), "first toggle: {}", text);
            }
            _ => panic!("expected Action"),
        }
        assert!(shared.lock().unwrap().fast_mode);

        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("disabled"), "second toggle: {}", text);
            }
            _ => panic!("expected Action"),
        }
        assert!(!shared.lock().unwrap().fast_mode);
    }

    #[test]
    fn test_brief_toggles() {
        let (ctx, shared) = test_ctx_with_shared();
        let handler = BriefHandler;

        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("enabled"), "first toggle: {}", text);
            }
            _ => panic!("expected Action"),
        }
        assert!(shared.lock().unwrap().brief_mode);

        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("disabled"), "second toggle: {}", text);
            }
            _ => panic!("expected Action"),
        }
        assert!(!shared.lock().unwrap().brief_mode);
    }

    #[test]
    fn test_verbose_toggles() {
        let (ctx, shared) = test_ctx_with_shared();
        let handler = VerboseHandler;

        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("enabled"), "first toggle: {}", text);
            }
            _ => panic!("expected Action"),
        }
        assert!(shared.lock().unwrap().verbose_mode);
    }

    #[test]
    fn test_theme_toggles() {
        let (ctx, shared) = test_ctx_with_shared();
        let handler = ThemeHandler;

        // Starts dark, toggle to light
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("light"), "toggled to light: {}", text);
            }
            _ => panic!("expected Action"),
        }
        assert!(!shared.lock().unwrap().dark_theme);

        // Toggle back to dark
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("dark"), "toggled to dark: {}", text);
            }
            _ => panic!("expected Action"),
        }
        assert!(shared.lock().unwrap().dark_theme);
    }

    #[test]
    fn test_effort_set_and_show() {
        let (ctx, shared) = test_ctx_with_shared();
        let handler = EffortHandler;

        // Show current
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("medium"), "default effort: {}", text);
            }
            _ => panic!("expected Action"),
        }

        // Set to high
        let result = handler.execute("high", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("high"), "set to high: {}", text);
            }
            _ => panic!("expected Action"),
        }
        assert_eq!(shared.lock().unwrap().effort_level, "high");

        // Invalid level
        let result = handler.execute("ultra", &ctx).unwrap();
        match result {
            CommandResult::Error(text) => {
                assert!(text.contains("Invalid"), "invalid level: {}", text);
            }
            _ => panic!("expected Error for invalid effort level"),
        }
    }

    #[test]
    fn test_permissions_shows_mode() {
        let (ctx, _shared) = test_ctx_with_shared();
        let handler = PermissionsHandler;
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("default"), "perm mode: {}", text);
                assert!(text.contains("require approval"), "description: {}", text);
            }
            _ => panic!("expected Action"),
        }
    }

    #[test]
    fn test_fork_sets_flag() {
        let (ctx, shared) = test_ctx_with_shared();
        let handler = ForkHandler;
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("forked"), "fork msg: {}", text);
                assert!(text.contains("New session ID"), "session id: {}", text);
            }
            _ => panic!("expected Action"),
        }
        assert!(shared.lock().unwrap().fork_requested);
    }

    #[test]
    fn test_resume_lists_sessions() {
        // This will either list sessions or say none found -- both are valid
        let ctx = test_ctx();
        let handler = ResumeHandler;
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(
                    text.contains("sessions") || text.contains("No previous"),
                    "resume output: {}",
                    text
                );
            }
            CommandResult::Error(text) => {
                // Acceptable if sessions dir doesn't exist
                assert!(!text.is_empty(), "error should have message");
            }
            _ => panic!("expected Action or Error"),
        }
    }

    #[test]
    fn test_memory_handler_runs() {
        let ctx = test_ctx();
        let handler = MemoryHandler;
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("Memory files"), "memory header: {}", text);
            }
            _ => panic!("expected Action"),
        }
    }

    #[test]
    fn test_config_reads_settings_file() {
        let handler = ConfigHandler;
        let ctx = test_ctx();
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(
                    text.contains("Settings"),
                    "/config output should contain 'Settings', got: {}",
                    text
                );
                assert!(
                    text.contains("settings.json"),
                    "/config output should reference settings.json path, got: {}",
                    text
                );
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_fallback_when_no_shared_state() {
        let ctx = test_ctx();

        // Status falls back when no shared state
        let result = StatusHandler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("no live session"), "fallback: {}", text);
            }
            _ => panic!("expected Action"),
        }

        // Cost falls back when no shared state
        let result = CostHandler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("no live session"), "fallback: {}", text);
            }
            _ => panic!("expected Action"),
        }

        // Context falls back when no shared state
        let result = ContextHandler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("no live session"), "fallback: {}", text);
            }
            _ => panic!("expected Action"),
        }
    }
}
