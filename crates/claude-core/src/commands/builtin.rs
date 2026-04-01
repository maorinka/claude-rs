use std::process::Command as ProcessCommand;

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
             /effort        - Set effort level\n\
             /doctor        - Run environment health checks\n\
             /diff          - Show git diff (staged + unstaged)\n\
             /export        - Export session to file\n\
             /mcp           - Manage MCP servers\n\
             /plugin        - Manage plugins\n\
             /skills        - List available skills\n\
             /agents        - List running agents\n\
             /rewind        - Revert recent file changes\n\
             /files         - List project files\n\
             /init          - Initialize Claude Code in project\n\
             /stats         - Show usage statistics\n\
             /env           - Show environment variables\n\
             /hooks         - List configured hooks\n\
             /session       - Session management\n\
             /copy          - Copy last response to clipboard\n\
             /pr-comments   - Analyze PR comments\n\
             /proactive     - Enable proactive mode\n\
             /ultrareview   - Deep code review\n\
             /share         - Share conversation as markdown\n\
             /usage         - Detailed token usage breakdown\n\
             /rename        - Rename current session\n\
             /add-dir       - Add working directories\n\
             /keybindings   - Show keyboard shortcuts\n\
             /reload-plugins - Reload plugin directory\n\
             /release-notes - Show release notes\n\
             /color         - Set session color\n\
             /sandbox       - Toggle sandbox mode\n\
             /output-style  - (deprecated) Use /config\n\
             /commit-push-pr - Commit, push, and create PR\n\
             /security-review - Security review of branch\n\
             /ultraplan     - Ultra-detailed planning mode\n\
             /thinkback     - Replay reasoning process\n\
             /insights      - Usage insights report"
                .to_string(),
        ))
    }
}

pub struct StatusHandler;
impl CommandHandler for StatusHandler {
    fn execute(&self, _args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        if let Some(ref shared) = ctx.shared {
            if let Ok(state) = shared.lock() {
                return Ok(CommandResult::Action(format!(
                    "Model: {}\nWorking directory: {}\nSession active\n\
                     Messages: {}\nTotal tokens: {}\nSession ID: {}\n\
                     API requests: {}",
                    ctx.model,
                    ctx.working_directory.display(),
                    state.message_count,
                    state.total_tokens,
                    state.session_id,
                    state.request_count,
                )));
            }
        }
        Ok(CommandResult::Action(format!(
            "Model: {}\nWorking directory: {}\nno live session data available",
            ctx.model,
            ctx.working_directory.display()
        )))
    }
}

pub struct ClearHandler;
impl CommandHandler for ClearHandler {
    fn execute(&self, _args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        if let Some(ref shared) = ctx.shared {
            if let Ok(mut state) = shared.lock() {
                state.clear_requested = true;
                state.message_count = 0;
                state.total_tokens = 0;
                state.request_count = 0;
            }
        }
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
            let new_model = args.trim().to_string();
            if let Some(ref shared) = ctx.shared {
                if let Ok(mut state) = shared.lock() {
                    state.model = new_model.clone();
                }
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
        let content =
            std::fs::read_to_string(&settings_path).unwrap_or_else(|_| "{}".into());
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
        if let Some(ref shared) = ctx.shared {
            if let Ok(state) = shared.lock() {
                if !state.cost_summary.is_empty() {
                    return Ok(CommandResult::Action(state.cost_summary.clone()));
                }
            }
        }
        Ok(CommandResult::Action(format!(
            "Model: {}\nno live session data available",
            ctx.model
        )))
    }
}

pub struct PermissionsHandler;
impl CommandHandler for PermissionsHandler {
    fn execute(&self, _args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        let mode = ctx
            .shared
            .as_ref()
            .and_then(|s| s.lock().ok().map(|st| st.permission_mode.clone()))
            .unwrap_or_else(|| "default".to_string());

        let desc = match mode.as_str() {
            "default" => "All tool calls require approval before execution.",
            "plan" => "Read-only tools are auto-approved; write tools require approval.",
            "auto-edit" => "File edits are auto-approved; shell commands require approval.",
            "yolo" | "dangerously-skip-permissions" => {
                "All tools auto-approved (dangerous)."
            }
            _ => "All tool calls require approval before execution.",
        };

        Ok(CommandResult::Action(format!(
            "Permission mode: {}\n{}",
            mode, desc
        )))
    }
}

pub struct VerboseHandler;
impl CommandHandler for VerboseHandler {
    fn execute(&self, _args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        if let Some(ref shared) = ctx.shared {
            if let Ok(mut state) = shared.lock() {
                state.verbose_mode = !state.verbose_mode;
                return Ok(CommandResult::Action(format!(
                    "Verbose mode {}.",
                    if state.verbose_mode { "enabled" } else { "disabled" }
                )));
            }
        }
        Ok(CommandResult::Action("Verbose mode toggled.".to_string()))
    }
}

pub struct MemoryHandler;
impl CommandHandler for MemoryHandler {
    fn execute(&self, _args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        let mut report = String::from("=== Memory files ===\n\n");

        // Global memory
        if let Ok(claude_home) = crate::config::paths::claude_dir() {
            let global_md = claude_home.join("CLAUDE.md");
            if global_md.exists() {
                report.push_str(&format!("  {} (global)\n", global_md.display()));
            } else {
                report.push_str("  ~/.claude/CLAUDE.md (global) - not found\n");
            }
        }

        // Project memory
        let project_md = ctx.working_directory.join("CLAUDE.md");
        if project_md.exists() {
            report.push_str(&format!("  {} (project)\n", project_md.display()));
        } else {
            report.push_str("  ./CLAUDE.md (project) - not found\n");
        }

        Ok(CommandResult::Action(report))
    }
}

pub struct TasksHandler;
impl CommandHandler for TasksHandler {
    fn execute(&self, _args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        Ok(CommandResult::Action("No active tasks.".to_string()))
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
    fn execute(&self, _args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        let new_id = uuid::Uuid::new_v4().to_string();
        if let Some(ref shared) = ctx.shared {
            if let Ok(mut state) = shared.lock() {
                state.fork_requested = true;
                state.session_id = new_id.clone();
            }
        }
        Ok(CommandResult::Action(format!(
            "Session forked. A new independent session has been created from this point.\n\
             New session ID: {}",
            new_id
        )))
    }
}

pub struct ContextHandler;
impl CommandHandler for ContextHandler {
    fn execute(&self, _args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        if let Some(ref shared) = ctx.shared {
            if let Ok(state) = shared.lock() {
                let used = state.total_tokens;
                let window = state.context_window;
                let pct = if window > 0 {
                    (used as f64 / window as f64 * 100.0) as u64
                } else {
                    0
                };
                return Ok(CommandResult::Action(format!(
                    "Context window usage:\n  Used:      {} tokens\n  Available: {} tokens\n  Utilization: {}%",
                    used, window, pct
                )));
            }
        }
        Ok(CommandResult::Action(
            "Context window usage:\nno live session data available".to_string(),
        ))
    }
}

pub struct ThemeHandler;
impl CommandHandler for ThemeHandler {
    fn execute(&self, _args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        if let Some(ref shared) = ctx.shared {
            if let Ok(mut state) = shared.lock() {
                state.dark_theme = !state.dark_theme;
                return Ok(CommandResult::Action(format!(
                    "Theme switched to {}.",
                    if state.dark_theme { "dark" } else { "light" }
                )));
            }
        }
        Ok(CommandResult::Action("Theme toggled.".to_string()))
    }
}

pub struct FastHandler;
impl CommandHandler for FastHandler {
    fn execute(&self, _args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        if let Some(ref shared) = ctx.shared {
            if let Ok(mut state) = shared.lock() {
                state.fast_mode = !state.fast_mode;
                return Ok(CommandResult::Action(format!(
                    "Fast mode {}.",
                    if state.fast_mode { "enabled" } else { "disabled" }
                )));
            }
        }
        Ok(CommandResult::Action("Fast mode toggled.".to_string()))
    }
}

pub struct BriefHandler;
impl CommandHandler for BriefHandler {
    fn execute(&self, _args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        if let Some(ref shared) = ctx.shared {
            if let Ok(mut state) = shared.lock() {
                state.brief_mode = !state.brief_mode;
                return Ok(CommandResult::Action(format!(
                    "Brief mode {}.",
                    if state.brief_mode { "enabled" } else { "disabled" }
                )));
            }
        }
        Ok(CommandResult::Action("Brief mode toggled.".to_string()))
    }
}

pub struct EffortHandler;
impl CommandHandler for EffortHandler {
    fn execute(&self, args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        let level = args.trim();
        if level.is_empty() {
            let current = ctx
                .shared
                .as_ref()
                .and_then(|s| s.lock().ok().map(|st| st.effort_level.clone()))
                .unwrap_or_else(|| "medium".to_string());
            Ok(CommandResult::Action(format!(
                "Usage: /effort <low|medium|high>. Current effort level: {}",
                current
            )))
        } else {
            match level {
                "low" | "medium" | "high" => {
                    if let Some(ref shared) = ctx.shared {
                        if let Ok(mut state) = shared.lock() {
                            state.effort_level = level.to_string();
                        }
                    }
                    Ok(CommandResult::Action(format!(
                        "Effort level set to: {}",
                        level
                    )))
                }
                _ => Ok(CommandResult::Error(format!(
                    "Invalid effort level: '{}'. Use low, medium, or high.",
                    level
                ))),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// New action-type command handlers
// ---------------------------------------------------------------------------

pub struct DoctorHandler;
impl CommandHandler for DoctorHandler {
    fn execute(&self, _args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        let mut report = String::from("=== Claude Code Doctor ===\n\n");

        let git_ok = ProcessCommand::new("git")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        report.push_str(&format!(
            "[{}] git: {}\n",
            if git_ok { "OK" } else { "FAIL" },
            if git_ok { "available" } else { "not found" }
        ));

        let rg_ok = ProcessCommand::new("rg")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        report.push_str(&format!(
            "[{}] ripgrep (rg): {}\n",
            if rg_ok { "OK" } else { "WARN" },
            if rg_ok {
                "available"
            } else {
                "not found (optional)"
            }
        ));

        let api_key_set = std::env::var("ANTHROPIC_API_KEY")
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        report.push_str(&format!(
            "[{}] ANTHROPIC_API_KEY: {}\n",
            if api_key_set { "OK" } else { "WARN" },
            if api_key_set {
                "set"
            } else {
                "not set (may use other auth)"
            }
        ));

        let settings_path = crate::config::paths::user_settings_path()
            .unwrap_or_else(|_| std::path::PathBuf::from("~/.claude/settings.json"));
        let settings_ok = settings_path.exists();
        report.push_str(&format!(
            "[{}] settings: {}\n",
            if settings_ok { "OK" } else { "INFO" },
            if settings_ok {
                format!("{}", settings_path.display())
            } else {
                "no settings file (using defaults)".to_string()
            }
        ));

        let wd_ok = ctx.working_directory.exists();
        report.push_str(&format!(
            "[{}] working directory: {}\n",
            if wd_ok { "OK" } else { "FAIL" },
            ctx.working_directory.display()
        ));

        let claude_dir = ctx.working_directory.join(".claude");
        let project_init = claude_dir.exists();
        report.push_str(&format!(
            "[{}] project .claude/: {}\n",
            if project_init { "OK" } else { "INFO" },
            if project_init {
                "present"
            } else {
                "not initialized (run /init)"
            }
        ));

        report.push_str(&format!("\nModel: {}\n", ctx.model));
        Ok(CommandResult::Action(report))
    }
}

pub struct DiffHandler;
impl CommandHandler for DiffHandler {
    fn execute(&self, _args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        let mut output = String::new();

        match ProcessCommand::new("git")
            .args(["diff"])
            .current_dir(&ctx.working_directory)
            .output()
        {
            Ok(result) if result.status.success() => {
                let diff = String::from_utf8_lossy(&result.stdout);
                if diff.trim().is_empty() {
                    output.push_str("No unstaged changes.\n");
                } else {
                    output.push_str("=== Unstaged Changes ===\n");
                    output.push_str(&diff);
                    output.push('\n');
                }
            }
            Ok(result) => {
                let stderr = String::from_utf8_lossy(&result.stderr);
                output.push_str(&format!("git diff failed: {}\n", stderr.trim()));
            }
            Err(e) => {
                output.push_str(&format!("Failed to run git diff: {}\n", e));
            }
        }

        match ProcessCommand::new("git")
            .args(["diff", "--cached"])
            .current_dir(&ctx.working_directory)
            .output()
        {
            Ok(result) if result.status.success() => {
                let diff = String::from_utf8_lossy(&result.stdout);
                if diff.trim().is_empty() {
                    output.push_str("No staged changes.\n");
                } else {
                    output.push_str("=== Staged Changes ===\n");
                    output.push_str(&diff);
                    output.push('\n');
                }
            }
            Ok(result) => {
                let stderr = String::from_utf8_lossy(&result.stderr);
                output.push_str(&format!(
                    "git diff --cached failed: {}\n",
                    stderr.trim()
                ));
            }
            Err(e) => {
                output.push_str(&format!(
                    "Failed to run git diff --cached: {}\n",
                    e
                ));
            }
        }

        Ok(CommandResult::Action(output))
    }
}

pub struct ExportHandler;
impl CommandHandler for ExportHandler {
    fn execute(&self, args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        let filename = if args.trim().is_empty() {
            let ts = chrono::Utc::now().format("%Y%m%d_%H%M%S");
            format!("claude_export_{}.md", ts)
        } else {
            args.trim().to_string()
        };

        let export_path = ctx.working_directory.join(&filename);

        let content = format!(
            "# Claude Code Session Export\n\n\
             Exported: {}\n\
             Model: {}\n\
             Working directory: {}\n\n\
             ---\n\n\
             (Session conversation would be exported here by the TUI layer.)\n",
            chrono::Utc::now().to_rfc3339(),
            ctx.model,
            ctx.working_directory.display(),
        );

        match std::fs::write(&export_path, &content) {
            Ok(()) => Ok(CommandResult::Action(format!(
                "Session exported to: {}",
                export_path.display()
            ))),
            Err(e) => Ok(CommandResult::Error(format!(
                "Failed to export session: {}",
                e
            ))),
        }
    }
}

pub struct McpHandler;
impl CommandHandler for McpHandler {
    fn execute(&self, args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        let sub = args.trim();
        if sub.is_empty() || sub == "list" {
            let settings_path = crate::config::paths::user_settings_path()
                .unwrap_or_else(|_| std::path::PathBuf::from("~/.claude/settings.json"));
            let content = std::fs::read_to_string(&settings_path)
                .unwrap_or_else(|_| "{}".into());

            let parsed: serde_json::Value = serde_json::from_str(&content)
                .unwrap_or(serde_json::Value::Object(Default::default()));

            let mut report = String::from("=== MCP Servers ===\n\n");

            if let Some(servers) =
                parsed.get("mcpServers").and_then(|v| v.as_object())
            {
                if servers.is_empty() {
                    report.push_str("No MCP servers configured.\n");
                } else {
                    for (name, config) in servers {
                        let enabled = config
                            .get("disabled")
                            .and_then(|v| v.as_bool())
                            .map(|d| !d)
                            .unwrap_or(true);
                        report.push_str(&format!(
                            "  {} [{}]\n",
                            name,
                            if enabled { "enabled" } else { "disabled" }
                        ));
                    }
                }
            } else {
                report.push_str("No MCP servers configured.\n");
            }

            report.push_str("\nUsage: /mcp [list|status]");
            Ok(CommandResult::Action(report))
        } else {
            Ok(CommandResult::Action(format!(
                "MCP subcommand '{}' is not yet implemented.",
                sub
            )))
        }
    }
}

pub struct PluginHandler;
impl CommandHandler for PluginHandler {
    fn execute(&self, _args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        let mut report = String::from("=== Installed Plugins ===\n\n");

        if let Ok(claude_home) = crate::config::paths::claude_dir() {
            let plugins_dir = claude_home.join("plugins");
            if plugins_dir.exists() {
                match std::fs::read_dir(&plugins_dir) {
                    Ok(entries) => {
                        let mut found = false;
                        for entry in entries.flatten() {
                            if entry.path().is_dir() {
                                let name = entry
                                    .file_name()
                                    .to_string_lossy()
                                    .to_string();
                                report.push_str(&format!("  - {}\n", name));
                                found = true;
                            }
                        }
                        if !found {
                            report.push_str("No plugins installed.\n");
                        }
                    }
                    Err(_) => {
                        report.push_str("No plugins installed.\n");
                    }
                }
            } else {
                report.push_str("No plugins installed.\n");
            }
        } else {
            report.push_str("Could not determine plugins directory.\n");
        }

        report.push_str("\nUse /plugin install <name> to install plugins.");
        Ok(CommandResult::Action(report))
    }
}

pub struct SkillsHandler;
impl CommandHandler for SkillsHandler {
    fn execute(&self, _args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        let project_root =
            crate::config::paths::detect_project_root(&ctx.working_directory);
        let skills = crate::plugins::skill::discover_skills(&project_root);

        let mut report = String::from("=== Available Skills ===\n\n");

        let builtin = crate::plugins::skill::builtin_skill_names();
        if !builtin.is_empty() {
            report.push_str("Builtin:\n");
            for name in &builtin {
                report.push_str(&format!("  /{}\n", name));
            }
        }

        if skills.is_empty() {
            report.push_str("\nNo project or user skills found.\n");
        } else {
            report.push_str("\nDiscovered:\n");
            for skill in &skills {
                report.push_str(&format!(
                    "  /{} - {}\n",
                    skill.name, skill.description
                ));
            }
        }

        report.push_str("\nSkills are defined in .claude/skills/<name>/SKILL.md");
        Ok(CommandResult::Action(report))
    }
}

pub struct AgentsHandler;
impl CommandHandler for AgentsHandler {
    fn execute(&self, _args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        Ok(CommandResult::Action(
            "=== Agents ===\n\n\
             No background agents currently running.\n\n\
             Use subagents within a conversation to run parallel tasks."
                .to_string(),
        ))
    }
}

pub struct RewindHandler;
impl CommandHandler for RewindHandler {
    fn execute(&self, _args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        Ok(CommandResult::Action(
            "Rewind requested.\n\
             The runtime will restore modified files to their previous state.\n\
             (If no file snapshots exist for this session, no changes are made.)"
                .to_string(),
        ))
    }
}

pub struct FilesHandler;
impl CommandHandler for FilesHandler {
    fn execute(&self, _args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        match ProcessCommand::new("git")
            .args(["ls-files"])
            .current_dir(&ctx.working_directory)
            .output()
        {
            Ok(output) if output.status.success() => {
                let files = String::from_utf8_lossy(&output.stdout);
                let lines: Vec<&str> = files.lines().collect();
                let count = lines.len();
                let display = if count > 50 {
                    let mut truncated: Vec<&str> = lines[..50].to_vec();
                    truncated.push("...");
                    format!(
                        "=== Project Files ({} total, showing first 50) ===\n\n{}",
                        count,
                        truncated.join("\n")
                    )
                } else {
                    format!(
                        "=== Project Files ({} total) ===\n\n{}",
                        count,
                        lines.join("\n")
                    )
                };
                Ok(CommandResult::Action(display))
            }
            _ => Ok(CommandResult::Action(format!(
                "Not a git repository. Working directory: {}",
                ctx.working_directory.display()
            ))),
        }
    }
}

pub struct InitHandler;
impl CommandHandler for InitHandler {
    fn execute(&self, _args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        let claude_dir = ctx.working_directory.join(".claude");
        let settings_file = claude_dir.join("settings.json");
        let claude_md = ctx.working_directory.join("CLAUDE.md");

        let mut actions = Vec::new();

        if !claude_dir.exists() {
            match std::fs::create_dir_all(&claude_dir) {
                Ok(()) => actions.push(format!("Created {}", claude_dir.display())),
                Err(e) => {
                    return Ok(CommandResult::Error(format!(
                        "Failed to create .claude/ directory: {}",
                        e
                    )));
                }
            }
        } else {
            actions.push(format!("{} already exists", claude_dir.display()));
        }

        if !settings_file.exists() {
            match std::fs::write(&settings_file, "{}") {
                Ok(()) => actions.push(format!("Created {}", settings_file.display())),
                Err(e) => actions.push(format!("Failed to create settings.json: {}", e)),
            }
        } else {
            actions.push(format!("{} already exists", settings_file.display()));
        }

        if !claude_md.exists() {
            let template = "# CLAUDE.md\n\n\
                            This file provides guidance to Claude Code (claude.ai/code) \
                            when working with code in this repository.\n";
            match std::fs::write(&claude_md, template) {
                Ok(()) => actions.push(format!("Created {}", claude_md.display())),
                Err(e) => actions.push(format!("Failed to create CLAUDE.md: {}", e)),
            }
        } else {
            actions.push(format!("{} already exists", claude_md.display()));
        }

        Ok(CommandResult::Action(format!(
            "=== Project Initialized ===\n\n{}",
            actions.join("\n")
        )))
    }
}

pub struct StatsHandler;
impl CommandHandler for StatsHandler {
    fn execute(&self, _args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        let mut report = String::from("=== Session Statistics ===\n\n");
        report.push_str(&format!("Model: {}\n", ctx.model));
        report.push_str(&format!(
            "Working directory: {}\n",
            ctx.working_directory.display()
        ));

        if let Some(ref shared) = ctx.shared {
            if let Ok(state) = shared.lock() {
                let elapsed = state.session_start.elapsed();
                let mins = elapsed.as_secs() / 60;
                let secs = elapsed.as_secs() % 60;
                report.push_str(&format!("Session duration: {}m {}s\n", mins, secs));
                report.push_str(&format!("Messages: {}\n", state.message_count));
                report.push_str(&format!("Tokens used: {}\n", state.total_tokens));
                report.push_str(&format!("API requests: {}\n", state.request_count));
                report.push_str(&format!("Total cost: ${:.4}\n", state.total_cost_usd));
                return Ok(CommandResult::Action(report));
            }
        }

        report.push_str("Session start: (current session)\n");
        report.push_str("Messages: (tracked by runtime)\n");
        report.push_str("Tokens used: (tracked by runtime)\n");
        Ok(CommandResult::Action(report))
    }
}

pub struct EnvHandler;
impl CommandHandler for EnvHandler {
    fn execute(&self, _args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        let mut report = String::from("=== Environment Variables ===\n\n");

        let vars: &[(&str, bool)] = &[
            ("ANTHROPIC_API_KEY", true),
            ("CLAUDE_RS_DEBUG", false),
            ("CLAUDE_CODE_USE_BEDROCK", false),
            ("CLAUDE_CODE_USE_VERTEX", false),
            ("CLAUDE_MODEL", false),
            ("CLAUDE_CONFIG_DIR", false),
            ("DISABLE_PROMPT_CACHING", false),
            ("HTTP_PROXY", false),
            ("HTTPS_PROXY", false),
        ];

        for (name, is_secret) in vars {
            match std::env::var(name) {
                Ok(val) => {
                    if *is_secret && !val.is_empty() {
                        report.push_str(&format!("  {} = [set]\n", name));
                    } else {
                        report.push_str(&format!("  {} = {}\n", name, val));
                    }
                }
                Err(_) => {
                    report.push_str(&format!("  {} = (not set)\n", name));
                }
            }
        }

        Ok(CommandResult::Action(report))
    }
}

pub struct HooksHandler;
impl CommandHandler for HooksHandler {
    fn execute(&self, _args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        let mut report = String::from("=== Configured Hooks ===\n\n");

        let settings_path = crate::config::paths::user_settings_path()
            .unwrap_or_else(|_| std::path::PathBuf::from("~/.claude/settings.json"));

        let project_settings = ctx
            .working_directory
            .join(".claude")
            .join("settings.json");

        let mut found_any = false;

        for (label, path) in [("User", settings_path), ("Project", project_settings)] {
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            let parsed: serde_json::Value = match serde_json::from_str(&content) {
                Ok(v) => v,
                Err(_) => continue,
            };

            if let Some(hooks) = parsed.get("hooks").and_then(|v| v.as_object()) {
                if !hooks.is_empty() {
                    report.push_str(&format!("{} hooks ({}):\n", label, path.display()));
                    for (event, config) in hooks {
                        report.push_str(&format!("  {} => {}\n", event, config));
                    }
                    report.push('\n');
                    found_any = true;
                }
            }
        }

        if !found_any {
            report.push_str("No hooks configured.\n\n");
        }

        report.push_str(
            "Hook events: PreToolUse, PostToolUse, PreSubmit, PostResponse, SessionStart, SessionEnd",
        );
        Ok(CommandResult::Action(report))
    }
}

pub struct SessionHandler;
impl CommandHandler for SessionHandler {
    fn execute(&self, _args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        let mut report = String::from("=== Session Info ===\n\n");

        report.push_str(&format!("Model: {}\n", ctx.model));
        report.push_str(&format!(
            "Working directory: {}\n",
            ctx.working_directory.display()
        ));

        if let Ok(sessions_dir) = crate::config::paths::sessions_dir() {
            if sessions_dir.exists() {
                match std::fs::read_dir(&sessions_dir) {
                    Ok(entries) => {
                        let mut sessions: Vec<String> = entries
                            .flatten()
                            .filter(|e| e.path().is_dir())
                            .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
                            .collect();
                        sessions.sort();
                        sessions.reverse();

                        let count = sessions.len();
                        report.push_str(&format!("\nStored sessions: {}\n", count));
                        for name in sessions.iter().take(5) {
                            report.push_str(&format!("  {}\n", name));
                        }
                        if count > 5 {
                            report.push_str(&format!("  ... and {} more\n", count - 5));
                        }
                    }
                    Err(_) => {
                        report.push_str("\nNo stored sessions.\n");
                    }
                }
            } else {
                report.push_str("\nNo stored sessions.\n");
            }
        }

        Ok(CommandResult::Action(report))
    }
}

pub struct CopyHandler;
impl CommandHandler for CopyHandler {
    fn execute(&self, _args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        let (cmd, cmd_args): (&str, &[&str]) = if cfg!(target_os = "macos") {
            ("pbcopy", &[])
        } else {
            ("xclip", &["-selection", "clipboard"])
        };

        let available = ProcessCommand::new("which")
            .arg(cmd)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        if !available {
            return Ok(CommandResult::Action(format!(
                "Clipboard tool '{}' not found. Cannot copy to clipboard.",
                cmd
            )));
        }

        // The TUI layer performs the actual copy; this handler signals intent.
        let _ = cmd_args;
        Ok(CommandResult::Action(format!(
            "Copy requested. Clipboard tool '{}' is available.\n\
             The runtime will copy the last assistant response to the clipboard.",
            cmd
        )))
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
                 for the task at hand. Do not execute any steps yet \u{2014} only produce the plan \
                 and wait for approval."
                    .to_string(),
            ))
        } else {
            Ok(CommandResult::Message(format!(
                "Enter plan mode: think carefully and produce a detailed, step-by-step plan \
                 for the following task. Do not execute any steps yet \u{2014} only produce the plan \
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
// New prompt-type command handlers
// ---------------------------------------------------------------------------

pub struct PrCommentsHandler;
impl CommandHandler for PrCommentsHandler {
    fn execute(&self, args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        let extra = if args.trim().is_empty() {
            String::new()
        } else {
            format!("\n\nAdditional context: {}", args.trim())
        };
        Ok(CommandResult::Message(format!(
            "Fetch and analyze the PR comments for the current branch.\n\n\
             Steps:\n\
             1. Use `gh pr view --json number,headRepository` to get PR info\n\
             2. Use `gh api` to fetch PR-level and review comments\n\
             3. Parse and format all comments showing author, file, line, diff hunk, and comment text\n\
             4. Suggest responses or actions for each comment thread\n\
             5. If there are no comments, report that{}",
            extra
        )))
    }
}

pub struct ProactiveHandler;
impl CommandHandler for ProactiveHandler {
    fn execute(&self, _args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        Ok(CommandResult::Message(
            "Enter proactive mode. Anticipate what I need and suggest actions before being \
             asked. Analyze the current context, recent changes, and project state to \
             proactively identify issues, suggest improvements, and propose next steps. \
             Continue offering proactive suggestions until I say to stop."
                .to_string(),
        ))
    }
}

pub struct UltrareviewHandler;
impl CommandHandler for UltrareviewHandler {
    fn execute(&self, args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        let scope = if args.trim().is_empty() {
            "the recent changes (run `git diff` to see them)".to_string()
        } else {
            format!("the following: \"{}\"", args.trim())
        };
        Ok(CommandResult::Message(format!(
            "Perform an extremely thorough, deep code review of {}.\n\n\
             Cover ALL of the following dimensions:\n\
             1. Correctness: logic errors, off-by-one, race conditions, null handling\n\
             2. Security: injection, auth issues, data exposure, input validation\n\
             3. Performance: unnecessary allocations, O(n^2) algorithms, missing caching\n\
             4. Error handling: missing error cases, swallowed errors, unhelpful messages\n\
             5. API design: naming, consistency, backwards compatibility\n\
             6. Testing: missing test cases, edge cases, test quality\n\
             7. Documentation: missing or outdated docs, unclear intent\n\
             8. Code style: readability, duplication, overly complex logic\n\
             9. Architecture: coupling, cohesion, SOLID violations\n\
             10. Dependencies: unnecessary deps, version issues, security advisories\n\n\
             Rate each dimension and provide specific, actionable feedback.",
            scope
        )))
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

    // Action commands (existing)
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

    // Action commands (new)
    register!("doctor",      "Run environment health checks",                   Action, DoctorHandler);
    register!("diff",        "Show git diff (staged + unstaged)",               Action, DiffHandler);
    register!("export",      "Export session to file",                          Action, ExportHandler);
    register!("mcp",         "Manage MCP servers",                              Action, McpHandler);
    register!("plugin",      "Manage plugins",                                  Action, PluginHandler);
    register!("skills",      "List available skills",                           Action, SkillsHandler);
    register!("agents",      "List running agents",                             Action, AgentsHandler);
    register!("rewind",      "Revert recent file changes",                      Action, RewindHandler);
    register!("files",       "List project files",                              Action, FilesHandler);
    register!("init",        "Initialize Claude Code in project",               Action, InitHandler);
    register!("stats",       "Show usage statistics",                           Action, StatsHandler);
    register!("env",         "Show environment variables",                      Action, EnvHandler);
    register!("hooks",       "List configured hooks",                           Action, HooksHandler);
    register!("session",     "Session management",                              Action, SessionHandler);
    register!("copy",        "Copy last response to clipboard",                 Action, CopyHandler);

    // Prompt commands (existing)
    register!("compact",      "Manually trigger conversation compaction",        Prompt, CompactHandler);
    register!("plan",         "Enter plan mode",                                 Prompt, PlanHandler);
    register!("exit-plan",    "Exit plan mode",                                  Prompt, ExitPlanHandler);
    register!("commit",       "Generate a git commit for staged changes",        Prompt, CommitHandler);
    register!("review",       "Review code changes (git diff)",                  Prompt, ReviewHandler);
    register!("branch",       "Create a new git branch",                         Prompt, BranchHandler);
    register!("pr",           "Create a pull request description",               Prompt, PrHandler);
    register!("bug",          "Report or analyze a bug",                         Prompt, BugHandler);
    register!("test",         "Generate tests for code",                         Prompt, TestHandler);
    register!("refactor",     "Suggest refactoring",                             Prompt, RefactorHandler);
    register!("explain",      "Explain code",                                    Prompt, ExplainHandler);
    register!("docs",         "Generate documentation",                          Prompt, DocsHandler);

    // Prompt commands (new)
    register!("pr-comments",  "Analyze PR comments",                             Prompt, PrCommentsHandler);
    register!("proactive",    "Enable proactive mode",                           Prompt, ProactiveHandler);
    register!("ultrareview",  "Deep code review",                                Prompt, UltrareviewHandler);

    registry
}

// ── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_ctx() -> CommandContext {
        CommandContext {
            working_directory: std::path::PathBuf::from("/tmp/test-project"),
            model: "claude-sonnet-4-20250514".to_string(),
            shared: None,
        }
    }

    fn test_ctx_with_dir(dir: &std::path::Path) -> CommandContext {
        CommandContext {
            working_directory: dir.to_path_buf(),
            model: "claude-sonnet-4-20250514".to_string(),
            shared: None,
        }
    }

    fn test_ctx_with_cost() -> CommandContext {
        use std::sync::{Arc, Mutex};
        let mut state = super::super::registry::SharedCommandState::default();
        state.cost_summary = "Total cost:            $0.0123\nTotal input tokens:    1000\nTotal output tokens:   500".to_string();
        state.model = "claude-sonnet-4-20250514".to_string();
        CommandContext {
            working_directory: std::path::PathBuf::from("/tmp/test-project"),
            model: "claude-sonnet-4-20250514".to_string(),
            shared: Some(Arc::new(Mutex::new(state))),
        }
    }

    #[test]
    fn test_cost_returns_model_name() {
        let handler = CostHandler;
        let ctx = test_ctx();
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(
                    text.contains(&ctx.model),
                    "/cost output should contain the model name, got: {}",
                    text
                );
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_cost_reads_from_shared_state() {
        let handler = CostHandler;
        let ctx = test_ctx_with_cost();
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(
                    text.contains("$0.0123"),
                    "/cost should show cost from shared state, got: {}",
                    text
                );
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_status_returns_working_directory() {
        let handler = StatusHandler;
        let ctx = test_ctx();
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("/tmp/test-project"));
                assert!(text.contains(&ctx.model));
                // Without shared state, shows fallback
                assert!(text.contains("no live session"));
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_config_reads_settings_file() {
        let handler = ConfigHandler;
        let ctx = test_ctx();
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("Settings"));
                assert!(text.contains("settings.json"));
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
    }

    // ------------------------------------------------------------------
    // Tests for new commands
    // ------------------------------------------------------------------

    #[test]
    fn test_doctor_returns_diagnostic_info() {
        let handler = DoctorHandler;
        let ctx = test_ctx();
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("Claude Code Doctor"));
                assert!(text.contains("git"));
                assert!(text.contains("ANTHROPIC_API_KEY"));
                assert!(text.contains("settings"));
                assert!(text.contains("working directory"));
                assert!(text.contains(&ctx.model));
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_diff_runs_git_command() {
        let handler = DiffHandler;
        let ctx = test_ctx();
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(!text.is_empty(), "/diff should produce output");
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_export_generates_file_path() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_ctx_with_dir(tmp.path());
        let handler = ExportHandler;
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("exported to") || text.contains("Session exported"));
                assert!(text.contains("claude_export_"));
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_export_custom_filename() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_ctx_with_dir(tmp.path());
        let handler = ExportHandler;
        let result = handler.execute("my_session.md", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("my_session.md"));
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_mcp_lists_servers() {
        let handler = McpHandler;
        let ctx = test_ctx();
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("MCP Servers"));
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_plugin_lists_plugins() {
        let handler = PluginHandler;
        let ctx = test_ctx();
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("Installed Plugins"));
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_skills_lists_builtin() {
        let handler = SkillsHandler;
        let ctx = test_ctx();
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("Available Skills"));
                assert!(text.contains("Builtin"));
                assert!(text.contains("/commit"));
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_init_creates_claude_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_ctx_with_dir(tmp.path());
        let handler = InitHandler;
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("Project Initialized"));
                assert!(tmp.path().join(".claude").exists());
                assert!(tmp.path().join(".claude/settings.json").exists());
                assert!(tmp.path().join("CLAUDE.md").exists());
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_init_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_ctx_with_dir(tmp.path());
        let handler = InitHandler;
        handler.execute("", &ctx).unwrap();
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("already exists"));
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_stats_returns_session_info() {
        let handler = StatsHandler;
        let ctx = test_ctx();
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("Session Statistics"));
                assert!(text.contains(&ctx.model));
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_stats_with_shared_state() {
        use std::sync::{Arc, Mutex};
        let shared = Arc::new(Mutex::new(super::super::registry::SharedCommandState {
            message_count: 42,
            total_tokens: 12345,
            request_count: 10,
            total_cost_usd: 0.0567,
            ..super::super::registry::SharedCommandState::default()
        }));
        let ctx = CommandContext {
            working_directory: std::path::PathBuf::from("/tmp/test"),
            model: "test-model".to_string(),
            shared: Some(shared),
        };
        let handler = StatsHandler;
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("42"), "should show message count");
                assert!(text.contains("12345"), "should show token count");
                assert!(text.contains("10"), "should show request count");
                assert!(text.contains("0.0567"), "should show cost");
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_env_shows_relevant_vars() {
        let handler = EnvHandler;
        let ctx = test_ctx();
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("Environment Variables"));
                assert!(text.contains("ANTHROPIC_API_KEY"));
                assert!(text.contains("CLAUDE_RS_DEBUG"));
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_hooks_shows_hook_events() {
        let handler = HooksHandler;
        let ctx = test_ctx();
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("Configured Hooks"));
                assert!(text.contains("PreToolUse"));
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_session_shows_model_and_directory() {
        let handler = SessionHandler;
        let ctx = test_ctx();
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("Session Info"));
                assert!(text.contains(&ctx.model));
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_copy_attempts_clipboard() {
        let handler = CopyHandler;
        let ctx = test_ctx();
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(
                    text.contains("pbcopy") || text.contains("xclip") || text.contains("Clipboard"),
                    "/copy should reference a clipboard tool, got: {}",
                    text
                );
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_agents_reports_no_agents() {
        let handler = AgentsHandler;
        let ctx = test_ctx();
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("Agents"));
                assert!(text.contains("No background agents"));
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_rewind_signals_intent() {
        let handler = RewindHandler;
        let ctx = test_ctx();
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("Rewind"));
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_files_produces_output() {
        let handler = FilesHandler;
        let ctx = test_ctx();
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(!text.is_empty(), "/files should produce output");
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_pr_comments_returns_prompt() {
        let handler = PrCommentsHandler;
        let ctx = test_ctx();
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Message(text) => {
                assert!(text.contains("PR comments"));
                assert!(text.contains("gh pr view"));
            }
            other => panic!("expected Message, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_proactive_returns_prompt() {
        let handler = ProactiveHandler;
        let ctx = test_ctx();
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Message(text) => {
                assert!(text.contains("proactive"));
            }
            other => panic!("expected Message, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_ultrareview_returns_prompt() {
        let handler = UltrareviewHandler;
        let ctx = test_ctx();
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Message(text) => {
                assert!(text.contains("thorough"));
                assert!(text.contains("Security"));
                assert!(text.contains("Performance"));
            }
            other => panic!("expected Message, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_registry_contains_all_new_commands() {
        let registry = build_default_commands();
        let expected = [
            "doctor", "diff", "export", "mcp", "plugin", "skills", "agents", "rewind",
            "files", "init", "stats", "env", "hooks", "session", "copy", "pr-comments",
            "proactive", "ultrareview",
        ];
        for name in &expected {
            assert!(
                registry.get(name).is_some(),
                "Registry should contain the '{}' command",
                name
            );
        }
    }

    #[test]
    fn test_help_lists_new_commands() {
        let handler = HelpHandler;
        let ctx = test_ctx();
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                for cmd in &[
                    "/doctor", "/diff", "/export", "/mcp", "/plugin", "/skills",
                    "/agents", "/rewind", "/files", "/init", "/stats", "/env",
                    "/hooks", "/session", "/copy", "/pr-comments", "/proactive",
                    "/ultrareview",
                ] {
                    assert!(text.contains(cmd), "/help should list {}", cmd);
                }
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
    }
}
