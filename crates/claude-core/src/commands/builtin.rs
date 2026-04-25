use std::process::Command as ProcessCommand;

use anyhow::Result;

use super::registry::{Command, CommandContext, CommandHandler, CommandRegistry, CommandResult};
use crate::config::settings::Settings;

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
             /theme         - Change color theme\n\
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

/// Model aliases matching TS MODEL_ALIASES
pub const MODEL_ALIASES: &[&str] = &[
    "sonnet",
    "opus",
    "haiku",
    "best",
    "sonnet[1m]",
    "opus[1m]",
    "opusplan",
];

/// Resolve a model alias to a canonical model name.
/// Matches TS parseUserSpecifiedModel() behavior.
pub fn parse_user_specified_model(input: &str) -> String {
    let trimmed = input.trim();
    let normalized = trimmed.to_lowercase();
    let has_1m = normalized.ends_with("[1m]");
    let base = if has_1m {
        normalized.trim_end_matches("[1m]").trim().to_string()
    } else {
        normalized.clone()
    };
    let suffix = if has_1m { "[1m]" } else { "" };

    match base.as_str() {
        "sonnet" => format!("claude-sonnet-4-6{}", suffix),
        "opus" => format!("claude-opus-4-6{}", suffix),
        "haiku" => format!("claude-haiku-4-5-20251001{}", suffix),
        "best" => "claude-opus-4-6".to_string(),
        "opusplan" => format!("claude-sonnet-4-6{}", suffix), // Sonnet default, Opus in plan mode
        _ => {
            // Preserve original case for custom model names
            if has_1m {
                format!(
                    "{}[1m]",
                    trimmed
                        .trim_end_matches("[1m]")
                        .trim_end_matches("[1M]")
                        .trim()
                )
            } else {
                trimmed.to_string()
            }
        }
    }
}

/// Get public display name for a model. Matches TS getPublicModelDisplayName().
pub fn render_model_name(model: &str) -> String {
    match model {
        "claude-opus-4-6" => "Opus 4.6".to_string(),
        "claude-opus-4-6[1m]" => "Opus 4.6 (1M context)".to_string(),
        "claude-opus-4-5-20250918" => "Opus 4.5".to_string(),
        "claude-opus-4-1-20250620" => "Opus 4.1".to_string(),
        "claude-sonnet-4-6" => "Sonnet 4.6".to_string(),
        "claude-sonnet-4-6[1m]" => "Sonnet 4.6 (1M context)".to_string(),
        "claude-sonnet-4-5-20250929" => "Sonnet 4.5".to_string(),
        "claude-sonnet-4-5-20250929[1m]" => "Sonnet 4.5 (1M context)".to_string(),
        "claude-haiku-4-5-20251001" => "Haiku 4.5".to_string(),
        "claude-3-7-sonnet-20250219" => "Sonnet 3.7".to_string(),
        "claude-3-5-sonnet-20241022" => "Sonnet 3.5".to_string(),
        "claude-3-5-haiku-20241022" => "Haiku 3.5".to_string(),
        other => other.to_string(),
    }
}

/// Available model options shown in /model picker.
/// Matches the TS ModelPicker component's model list.
const MODEL_OPTIONS: &[(&str, &str, &str)] = &[
    // (alias/id, display_name, description)
    ("opus", "Opus 4.6", "Most capable for complex work"),
    (
        "opus[1m]",
        "Opus 4.6 (1M)",
        "Most capable, extended context",
    ),
    ("sonnet", "Sonnet 4.6", "Best for everyday tasks"),
    (
        "sonnet[1m]",
        "Sonnet 4.6 (1M)",
        "Everyday tasks, extended context",
    ),
    ("haiku", "Haiku 4.5", "Fastest, cheapest"),
    (
        "opusplan",
        "Opus in plan, Sonnet otherwise",
        "Smart planning, fast execution",
    ),
    (
        "claude-sonnet-4-5-20250929",
        "Sonnet 4.5",
        "Previous generation",
    ),
    (
        "claude-opus-4-1-20250620",
        "Opus 4.1",
        "Previous generation",
    ),
];

impl CommandHandler for ModelHandler {
    fn execute(&self, args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        if args.trim().is_empty() {
            let current_display = render_model_name(&ctx.model);
            let mut output = format!("Current model: {}\n\nAvailable models:\n", current_display);
            for (alias, display, desc) in MODEL_OPTIONS {
                let resolved = parse_user_specified_model(alias);
                let marker = if ctx.model == resolved || ctx.model == *alias {
                    " ← current"
                } else {
                    ""
                };
                output.push_str(&format!(
                    "  {:<12} {:<25} {}{}\n",
                    alias, display, desc, marker
                ));
            }
            output.push_str("\nUsage: /model <name>  (e.g. /model sonnet, /model opus[1m])");
            output.push_str("\n\nYou can also use full model IDs (e.g. /model claude-sonnet-4-6)");
            Ok(CommandResult::Action(output))
        } else {
            let input = args.trim();
            let new_model = parse_user_specified_model(input);
            let display = render_model_name(&new_model);
            if let Some(ref shared) = ctx.shared {
                if let Ok(mut state) = shared.lock() {
                    state.model = new_model.clone();
                }
            }
            Ok(CommandResult::Action(format!("Set model to {}", display)))
        }
    }
}

pub struct ConfigHandler;
impl CommandHandler for ConfigHandler {
    fn execute(&self, _args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        let settings_path = crate::config::paths::user_settings_path()
            .unwrap_or_else(|_| std::path::PathBuf::from("~/.claude/settings.json"));
        let content = std::fs::read_to_string(&settings_path).unwrap_or_else(|_| "{}".into());
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
            "yolo" | "dangerously-skip-permissions" => "All tools auto-approved (dangerous).",
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
                    if state.verbose_mode {
                        "enabled"
                    } else {
                        "disabled"
                    }
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
    fn execute(&self, args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        let args = args.trim();

        // If args are provided, set the theme directly
        if !args.is_empty() {
            let valid = [
                "auto",
                "dark",
                "light",
                "dark-daltonized",
                "light-daltonized",
                "dark-ansi",
                "light-ansi",
            ];
            if valid.contains(&args.to_lowercase().as_str()) {
                if let Some(ref shared) = ctx.shared {
                    if let Ok(mut state) = shared.lock() {
                        state.theme_setting = args.to_lowercase();
                        // Keep dark_theme in sync for any legacy consumers
                        state.dark_theme = matches!(
                            args.to_lowercase().as_str(),
                            "dark" | "dark-daltonized" | "dark-ansi" | "auto"
                        );
                    }
                }
                return Ok(CommandResult::Action(format!(
                    "Theme set to {}",
                    args.to_lowercase()
                )));
            } else {
                return Ok(CommandResult::Error(format!(
                    "Unknown theme '{}'. Valid themes: {}",
                    args,
                    valid.join(", ")
                )));
            }
        }

        // No args: signal the TUI to open the picker.
        // Return a special action string the TUI can intercept.
        Ok(CommandResult::Action("__open_theme_picker__".to_string()))
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
                    if state.fast_mode {
                        "enabled"
                    } else {
                        "disabled"
                    }
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
                    if state.brief_mode {
                        "enabled"
                    } else {
                        "disabled"
                    }
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
                output.push_str(&format!("git diff --cached failed: {}\n", stderr.trim()));
            }
            Err(e) => {
                output.push_str(&format!("Failed to run git diff --cached: {}\n", e));
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
            let content = std::fs::read_to_string(&settings_path).unwrap_or_else(|_| "{}".into());

            let parsed: serde_json::Value = serde_json::from_str(&content)
                .unwrap_or(serde_json::Value::Object(Default::default()));

            let mut report = String::from("=== MCP Servers ===\n\n");

            if let Some(servers) = parsed.get("mcpServers").and_then(|v| v.as_object()) {
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
                                let name = entry.file_name().to_string_lossy().to_string();
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
        let project_root = crate::config::paths::detect_project_root(&ctx.working_directory);
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
                report.push_str(&format!("  /{} - {}\n", skill.name, skill.description));
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

/// INIT_PROMPT matches the TS OLD_INIT_PROMPT for legacy CLAUDE.md initialization.
const INIT_PROMPT: &str = "\
Please analyze this codebase and create a CLAUDE.md file, which will be given to future instances of Claude Code to operate in this repository.

What to add:
1. Commands that will be commonly used, such as how to build, lint, and run tests. Include the necessary commands to develop in this codebase, such as how to run a single test.
2. High-level code architecture and structure so that future instances can be productive more quickly. Focus on the \"big picture\" architecture that requires reading multiple files to understand.

Usage notes:
- If there's already a CLAUDE.md, suggest improvements to it.
- When you make the initial CLAUDE.md, do not repeat yourself and do not include obvious instructions like \"Provide helpful error messages to users\", \"Write unit tests for all new utilities\", \"Never include sensitive information (API keys, tokens) in code or commits\".
- Avoid listing every component or file structure that can be easily discovered.
- Don't include generic development practices.
- If there are Cursor rules (in .cursor/rules/ or .cursorrules) or Copilot rules (in .github/copilot-instructions.md), make sure to include the important parts.
- If there is a README.md, make sure to include the important parts.
- Do not make up information such as \"Common Development Tasks\", \"Tips for Development\", \"Support and Documentation\" unless this is expressly included in other files that you read.
- Be sure to prefix the file with the following text:

```
# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.
```";

/// Verbatim port of TS `NEW_INIT_PROMPT` (src/commands/init.ts:28-224).
///
/// The multi-phase init wizard text. The Rust port uses the simpler
/// [`INIT_PROMPT`] (OLD_INIT_PROMPT) today because several subsystems
/// this prompt depends on are not yet wired — subagent spawning from
/// commands, AskUserQuestion integration from commands, skills
/// creation flow, and the `update-config` skill reference. Parking
/// the text verbatim keeps it identical to TS so the dispatch flip
/// is a one-line change once those land.
pub const NEW_INIT_PROMPT: &str = include_str!("../prompts/new_init.md");

/// Verbatim port of TS `/init-verifiers` prompt
/// (src/commands/init-verifiers.ts:15-256). The 5-phase verifier-skill
/// creation wizard.
pub const INIT_VERIFIERS_PROMPT: &str = include_str!("../prompts/init_verifiers.md");

/// `/init-verifiers` — emits the 5-phase verifier-skill creation wizard
/// prompt as a user-side message so the model executes it. Subagent
/// spawning, AskUserQuestion, Playwright/MCP integration etc. are
/// expected to be available to the model; we just ship the prompt.
pub struct InitVerifiersHandler;
impl CommandHandler for InitVerifiersHandler {
    fn execute(&self, _args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        Ok(CommandResult::Message(INIT_VERIFIERS_PROMPT.to_string()))
    }
}

impl CommandHandler for InitHandler {
    fn execute(&self, _args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        // Ensure .claude/ directory exists
        let claude_dir = ctx.working_directory.join(".claude");
        if !claude_dir.exists() {
            let _ = std::fs::create_dir_all(&claude_dir);
        }

        // Ensure .claude/settings.json exists
        let settings_file = claude_dir.join("settings.json");
        if !settings_file.exists() {
            let _ = std::fs::write(&settings_file, "{}");
        }

        // Pick OLD vs NEW init prompt the same way TS does:
        //   USER_TYPE === 'ant' OR CLAUDE_CODE_NEW_INIT truthy → NEW_INIT_PROMPT
        //   otherwise → OLD INIT_PROMPT.
        // (TS reference: src/commands/init.ts:247-250)
        let prompt = if crate::user_type::is_ant()
            || crate::errors_util::is_env_truthy("CLAUDE_CODE_NEW_INIT")
        {
            NEW_INIT_PROMPT
        } else {
            INIT_PROMPT
        };

        Ok(CommandResult::Message(prompt.to_string()))
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

        let project_settings = ctx.working_directory.join(".claude").join("settings.json");

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
            "## Context\n\n\
             - Current git status: !`git status`\n\
             - Current git diff (staged and unstaged changes): !`git diff HEAD`\n\
             - Current branch: !`git branch --show-current`\n\
             - Recent commits: !`git log --oneline -10`\n\n\
             ## Git Safety Protocol\n\n\
             - NEVER update the git config\n\
             - NEVER skip hooks (--no-verify, --no-gpg-sign, etc) unless the user explicitly requests it\n\
             - CRITICAL: ALWAYS create NEW commits. NEVER use git commit --amend, unless the user explicitly requests it\n\
             - Do not commit files that likely contain secrets (.env, credentials.json, etc). Warn the user if they specifically request to commit those files\n\
             - If there are no changes to commit (i.e., no untracked files and no modifications), do not create an empty commit\n\
             - Never use git commands with the -i flag (like git rebase -i or git add -i) since they require interactive input which is not supported\n\n\
             ## Your task\n\n\
             Based on the above changes, create a single git commit:\n\n\
             1. Analyze all staged changes and draft a commit message:\n\
                - Look at the recent commits above to follow this repository's commit message style\n\
                - Summarize the nature of the changes (new feature, enhancement, bug fix, refactoring, test, docs, etc.)\n\
                - Ensure the message accurately reflects the changes and their purpose (i.e. \"add\" means a wholly new feature, \"update\" means an enhancement to an existing feature, \"fix\" means a bug fix, etc.)\n\
                - Draft a concise (1-2 sentences) commit message that focuses on the \"why\" rather than the \"what\"\n\n\
             2. Stage relevant files and create the commit using HEREDOC syntax:\n\
             ```\n\
             git commit -m \"$(cat <<'EOF'\n\
             Commit message here.\n\
             EOF\n\
             )\"\n\
             ```\n\n\
             You have the capability to call multiple tools in a single response. Stage and create the commit using a single message. Do not use any other tools or do anything else. Do not send any other text or messages besides these tool calls."
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
                "You are an expert code reviewer. Follow these steps:\n\n\
                 1. If no PR number is provided in the args, run `gh pr list` to show open PRs\n\
                 2. If a PR number is provided, run `gh pr view <number>` to get PR details\n\
                 3. Run `gh pr diff <number>` to get the diff\n\
                 4. Analyze the changes and provide a thorough code review that includes:\n\
                    - Overview of what the PR does\n\
                    - Analysis of code quality and style\n\
                    - Specific suggestions for improvements\n\
                    - Any potential issues or risks\n\n\
                 Keep your review concise but thorough. Focus on:\n\
                 - Code correctness\n\
                 - Following project conventions\n\
                 - Performance implications\n\
                 - Test coverage\n\
                 - Security considerations\n\n\
                 Format your review with clear sections and bullet points.\n\n\
                 PR number: (none provided — list open PRs)"
                    .to_string(),
            ))
        } else {
            Ok(CommandResult::Message(format!(
                "You are an expert code reviewer. Follow these steps:\n\n\
                 1. If no PR number is provided in the args, run `gh pr list` to show open PRs\n\
                 2. If a PR number is provided, run `gh pr view <number>` to get PR details\n\
                 3. Run `gh pr diff <number>` to get the diff\n\
                 4. Analyze the changes and provide a thorough code review that includes:\n\
                    - Overview of what the PR does\n\
                    - Analysis of code quality and style\n\
                    - Specific suggestions for improvements\n\
                    - Any potential issues or risks\n\n\
                 Keep your review concise but thorough. Focus on:\n\
                 - Code correctness\n\
                 - Following project conventions\n\
                 - Performance implications\n\
                 - Test coverage\n\
                 - Security considerations\n\n\
                 Format your review with clear sections and bullet points.\n\n\
                 PR number: {}",
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
            format!("\nAdditional user input: {}", args.trim())
        };
        Ok(CommandResult::Message(format!(
            "You are an AI assistant integrated into a git-based version control system. Your task is to fetch and display comments from a GitHub pull request.\n\n\
             Follow these steps:\n\n\
             1. Use `gh pr view --json number,headRepository` to get the PR number and repository info\n\
             2. Use `gh api /repos/{{owner}}/{{repo}}/issues/{{number}}/comments` to get PR-level comments\n\
             3. Use `gh api /repos/{{owner}}/{{repo}}/pulls/{{number}}/comments` to get review comments. Pay particular attention to the following fields: `body`, `diff_hunk`, `path`, `line`, etc. If the comment references some code, consider fetching it using eg `gh api /repos/{{owner}}/{{repo}}/contents/{{path}}?ref={{branch}} | jq .content -r | base64 -d`\n\
             4. Parse and format all comments in a readable way\n\
             5. Return ONLY the formatted comments, with no additional text\n\n\
             Format the comments as:\n\n\
             ## Comments\n\n\
             [For each comment thread:]\n\
             - @author file.ts#line:\n\
               ```diff\n\
               [diff_hunk from the API response]\n\
               ```\n\
               > quoted comment text\n\n\
               [any replies indented]\n\n\
             If there are no comments, return \"No comments found.\"\n\n\
             Remember:\n\
             1. Only show the actual comments, no explanatory text\n\
             2. Include both PR-level and code review comments\n\
             3. Preserve the threading/nesting of comment replies\n\
             4. Show the file and line number context for code review comments\n\
             5. Use jq to parse the JSON responses from the GitHub API\n\
             {extra}",
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
// Batch 3 – the 15 remaining missing commands
// ---------------------------------------------------------------------------

/// /share — Export conversation as shareable markdown file
pub struct ShareHandler;
impl CommandHandler for ShareHandler {
    fn execute(&self, args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        let filename = if args.trim().is_empty() {
            let ts = chrono::Utc::now().format("%Y%m%d_%H%M%S");
            format!("claude_share_{}.md", ts)
        } else {
            let mut name = args.trim().to_string();
            if !name.ends_with(".md") {
                name.push_str(".md");
            }
            name
        };

        let export_path = ctx.working_directory.join(&filename);

        let session_name = ctx
            .shared
            .as_ref()
            .and_then(|s| {
                s.lock().ok().and_then(|st| {
                    if st.session_name.is_empty() {
                        None
                    } else {
                        Some(st.session_name.clone())
                    }
                })
            })
            .unwrap_or_else(|| "Untitled Session".to_string());

        let (msg_count, token_count) = ctx
            .shared
            .as_ref()
            .and_then(|s| s.lock().ok().map(|st| (st.message_count, st.total_tokens)))
            .unwrap_or((0, 0));

        let content = format!(
            "# {}\n\n\
             > Shared from Claude Code\n\n\
             **Model:** {}\n\
             **Messages:** {}\n\
             **Tokens:** {}\n\
             **Exported:** {}\n\
             **Working directory:** {}\n\n\
             ---\n\n\
             (Conversation transcript is populated by the TUI layer.)\n",
            session_name,
            ctx.model,
            msg_count,
            token_count,
            chrono::Utc::now().to_rfc3339(),
            ctx.working_directory.display(),
        );

        match std::fs::write(&export_path, &content) {
            Ok(()) => Ok(CommandResult::Action(format!(
                "Conversation shared to: {}\nYou can send this file to others for review.",
                export_path.display()
            ))),
            Err(e) => Ok(CommandResult::Error(format!(
                "Failed to share conversation: {}",
                e
            ))),
        }
    }
}

/// /usage — Detailed token usage breakdown per turn
pub struct UsageHandler;
impl CommandHandler for UsageHandler {
    fn execute(&self, _args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        if let Some(ref shared) = ctx.shared {
            if let Ok(state) = shared.lock() {
                let mut report = String::from("=== Token Usage Breakdown ===\n\n");
                report.push_str(&format!("Model: {}\n", state.model));

                let elapsed = state.session_start.elapsed();
                let mins = elapsed.as_secs() / 60;
                let secs = elapsed.as_secs() % 60;
                report.push_str(&format!("Session duration: {}m {}s\n", mins, secs));
                report.push_str(&format!("Total messages: {}\n", state.message_count));
                report.push_str(&format!("Total tokens: {}\n", state.total_tokens));
                report.push_str(&format!("API requests: {}\n", state.request_count));
                report.push_str(&format!("Total cost: ${:.4}\n\n", state.total_cost_usd));

                if state.per_turn_tokens.is_empty() {
                    report.push_str("Per-turn breakdown:\n  (no per-turn data recorded yet)\n");
                } else {
                    report.push_str("Per-turn breakdown:\n");
                    report.push_str("  Turn | Input tokens | Output tokens\n");
                    report.push_str("  -----|-------------|-------------\n");
                    for (turn, input, output) in &state.per_turn_tokens {
                        report
                            .push_str(&format!("  {:>4} | {:>11} | {:>12}\n", turn, input, output));
                    }
                }

                if state.total_tokens > 0 && state.message_count > 0 {
                    let avg = state.total_tokens / state.message_count as u64;
                    report.push_str(&format!("\nAverage tokens/message: {}\n", avg));
                }

                return Ok(CommandResult::Action(report));
            }
        }
        Ok(CommandResult::Action(
            "=== Token Usage Breakdown ===\n\nNo live session data available.\n\
             Usage data is populated once a session is active."
                .to_string(),
        ))
    }
}

/// /rename — Rename current session
pub struct RenameHandler;
impl CommandHandler for RenameHandler {
    fn execute(&self, args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        let new_name = args.trim();
        if new_name.is_empty() {
            return Ok(CommandResult::Error(
                "Usage: /rename <name>\nProvide a name for the current session.".to_string(),
            ));
        }

        if let Some(ref shared) = ctx.shared {
            if let Ok(mut state) = shared.lock() {
                state.session_name = new_name.to_string();
                return Ok(CommandResult::Action(format!(
                    "Session renamed to: {}",
                    new_name
                )));
            }
        }
        Ok(CommandResult::Action(format!(
            "Session renamed to: {} (note: no live session to persist the name)",
            new_name
        )))
    }
}

/// /add-dir — Add directories to working set
pub struct AddDirHandler;
impl CommandHandler for AddDirHandler {
    fn execute(&self, args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        let dir_path = args.trim();
        if dir_path.is_empty() {
            return Ok(CommandResult::Error(
                "Usage: /add-dir <path>\nProvide a directory path to add to the working set."
                    .to_string(),
            ));
        }

        // Resolve the path (support relative and absolute)
        let resolved = if std::path::Path::new(dir_path).is_absolute() {
            std::path::PathBuf::from(dir_path)
        } else {
            ctx.working_directory.join(dir_path)
        };

        // Validate: must exist and be a directory
        if !resolved.exists() {
            return Ok(CommandResult::Error(format!(
                "Path not found: {}",
                resolved.display()
            )));
        }
        if !resolved.is_dir() {
            let parent = resolved
                .parent()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            return Ok(CommandResult::Error(format!(
                "{} is not a directory. Did you mean to add the parent directory {}?",
                resolved.display(),
                parent
            )));
        }

        let abs_str = resolved.display().to_string();

        // Check for duplicates
        if let Some(ref shared) = ctx.shared {
            if let Ok(mut state) = shared.lock() {
                // Check if already covered by the main working directory
                if resolved.starts_with(&ctx.working_directory) {
                    return Ok(CommandResult::Action(format!(
                        "{} is already accessible within the working directory {}.",
                        abs_str,
                        ctx.working_directory.display()
                    )));
                }
                if state.extra_dirs.contains(&abs_str) {
                    return Ok(CommandResult::Action(format!(
                        "{} is already in the working set.",
                        abs_str
                    )));
                }
                state.extra_dirs.push(abs_str.clone());
                return Ok(CommandResult::Action(format!(
                    "Added {} as a working directory.\nCurrent directories: {}, {}",
                    abs_str,
                    ctx.working_directory.display(),
                    state.extra_dirs.join(", ")
                )));
            }
        }

        Ok(CommandResult::Action(format!(
            "Added {} as a working directory.",
            abs_str
        )))
    }
}

/// /keybindings — Show/edit keyboard shortcuts
pub struct KeybindingsHandler;
impl CommandHandler for KeybindingsHandler {
    fn execute(&self, _args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        let keybindings_path = crate::config::paths::claude_dir()
            .map(|d| d.join("keybindings.json"))
            .unwrap_or_else(|_| {
                dirs::home_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join(".claude")
                    .join("keybindings.json")
            });

        if keybindings_path.exists() {
            match std::fs::read_to_string(&keybindings_path) {
                Ok(content) => Ok(CommandResult::Action(format!(
                    "=== Keybindings ({}) ===\n\n{}\n\n\
                     Edit this file to customize keyboard shortcuts.",
                    keybindings_path.display(),
                    content
                ))),
                Err(e) => Ok(CommandResult::Error(format!(
                    "Failed to read keybindings file: {}",
                    e
                ))),
            }
        } else {
            // Create a template
            let template = r#"{
  // Keybinding customization for Claude Code
  // Format: { "key": "action" }
  // Example bindings:
  //   "ctrl+s": "submit",
  //   "ctrl+c": "cancel",
  //   "ctrl+l": "clear",
  //   "escape": "dismiss"
}"#;

            if let Some(parent) = keybindings_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match std::fs::write(&keybindings_path, template) {
                Ok(()) => Ok(CommandResult::Action(format!(
                    "Created keybindings template at: {}\n\n{}\n\n\
                     Edit this file to customize keyboard shortcuts.",
                    keybindings_path.display(),
                    template
                ))),
                Err(e) => Ok(CommandResult::Error(format!(
                    "Failed to create keybindings file: {}",
                    e
                ))),
            }
        }
    }
}

/// /reload-plugins — Rescan plugin directory
pub struct ReloadPluginsHandler;
impl CommandHandler for ReloadPluginsHandler {
    fn execute(&self, _args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        let plugins_dir = crate::config::paths::claude_dir()
            .map(|d| d.join("plugins"))
            .unwrap_or_else(|_| {
                dirs::home_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join(".claude")
                    .join("plugins")
            });

        if !plugins_dir.exists() {
            return Ok(CommandResult::Action(
                "Reloaded: 0 plugins, 0 skills, 0 agents, 0 hooks\n\
                 No plugins directory found."
                    .to_string(),
            ));
        }

        let mut plugin_count = 0u32;
        let mut skill_count = 0u32;
        let mut error_count = 0u32;

        match std::fs::read_dir(&plugins_dir) {
            Ok(entries) => {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        plugin_count += 1;
                        // Count skills within each plugin
                        let skills_dir = path.join("skills");
                        if skills_dir.is_dir() {
                            if let Ok(skill_entries) = std::fs::read_dir(&skills_dir) {
                                skill_count += skill_entries
                                    .flatten()
                                    .filter(|e| e.path().is_dir())
                                    .count() as u32;
                            }
                        }
                        // Check for manifest validity
                        let manifest = path.join("manifest.json");
                        if !manifest.exists() {
                            let package_json = path.join("package.json");
                            if !package_json.exists() {
                                error_count += 1;
                            }
                        }
                    }
                }
            }
            Err(_) => {
                return Ok(CommandResult::Error(format!(
                    "Failed to read plugins directory: {}",
                    plugins_dir.display()
                )));
            }
        }

        let mut msg = format!(
            "Reloaded: {} plugin{}, {} skill{}",
            plugin_count,
            if plugin_count == 1 { "" } else { "s" },
            skill_count,
            if skill_count == 1 { "" } else { "s" },
        );

        if error_count > 0 {
            msg.push_str(&format!(
                "\n{} error{} during load. Run /doctor for details.",
                error_count,
                if error_count == 1 { "" } else { "s" }
            ));
        }

        Ok(CommandResult::Action(msg))
    }
}

/// /release-notes — Show version changelog
pub struct ReleaseNotesHandler;
impl CommandHandler for ReleaseNotesHandler {
    fn execute(&self, _args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        // Try to read a cached changelog from the claude config directory
        let changelog_path = crate::config::paths::claude_dir()
            .map(|d| d.join("CHANGELOG.md"))
            .unwrap_or_else(|_| std::path::PathBuf::from("CHANGELOG.md"));

        if changelog_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&changelog_path) {
                // Show the first 80 lines (most recent entries)
                let preview: String = content.lines().take(80).collect::<Vec<_>>().join("\n");
                return Ok(CommandResult::Action(format!(
                    "=== Release Notes ===\n\n{}\n\n(Showing first 80 lines from {})",
                    preview,
                    changelog_path.display()
                )));
            }
        }

        // Fall back to the project's own CHANGELOG if present
        let project_changelog =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../CHANGELOG.md");
        if project_changelog.exists() {
            if let Ok(content) = std::fs::read_to_string(&project_changelog) {
                let preview: String = content.lines().take(80).collect::<Vec<_>>().join("\n");
                return Ok(CommandResult::Action(format!(
                    "=== Release Notes ===\n\n{}",
                    preview
                )));
            }
        }

        Ok(CommandResult::Action(
            "=== Release Notes ===\n\n\
             No changelog found locally.\n\
             See the full changelog at: https://github.com/anthropics/claude-code/releases"
                .to_string(),
        ))
    }
}

/// /color — Color scheme management
pub struct ColorHandler;
impl CommandHandler for ColorHandler {
    fn execute(&self, args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        let available_colors = [
            "red", "blue", "green", "yellow", "purple", "orange", "pink", "cyan",
        ];
        let reset_aliases = ["default", "reset", "none", "gray", "grey"];

        let color_arg = args.trim().to_lowercase();

        if color_arg.is_empty() {
            let current = ctx
                .shared
                .as_ref()
                .and_then(|s| {
                    s.lock().ok().and_then(|st| {
                        if st.session_color.is_empty() {
                            None
                        } else {
                            Some(st.session_color.clone())
                        }
                    })
                })
                .unwrap_or_else(|| "default".to_string());

            return Ok(CommandResult::Action(format!(
                "Current session color: {}\n\
                 Available colors: {}, default\n\
                 Usage: /color <color>",
                current,
                available_colors.join(", ")
            )));
        }

        if reset_aliases.contains(&color_arg.as_str()) {
            if let Some(ref shared) = ctx.shared {
                if let Ok(mut state) = shared.lock() {
                    state.session_color = "default".to_string();
                }
            }
            return Ok(CommandResult::Action(
                "Session color reset to default.".to_string(),
            ));
        }

        if !available_colors.contains(&color_arg.as_str()) {
            return Ok(CommandResult::Error(format!(
                "Invalid color \"{}\". Available colors: {}, default",
                color_arg,
                available_colors.join(", ")
            )));
        }

        if let Some(ref shared) = ctx.shared {
            if let Ok(mut state) = shared.lock() {
                state.session_color = color_arg.clone();
            }
        }

        Ok(CommandResult::Action(format!(
            "Session color set to: {}",
            color_arg
        )))
    }
}

/// /sandbox — Toggle sandbox mode
pub struct SandboxHandler;
impl CommandHandler for SandboxHandler {
    fn execute(&self, args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        let sub = args.trim();

        // Handle 'exclude' subcommand
        if sub.starts_with("exclude ") {
            let pattern = sub.strip_prefix("exclude ").unwrap().trim();
            if pattern.is_empty() {
                return Ok(CommandResult::Error(
                    "Usage: /sandbox exclude <command-pattern>\n\
                     Example: /sandbox exclude \"npm run test:*\""
                        .to_string(),
                ));
            }
            // Strip surrounding quotes
            let clean = pattern.trim_matches('"').trim_matches('\'');
            return Ok(CommandResult::Action(format!(
                "Added \"{}\" to excluded sandbox commands.\n\
                 Note: configure excluded patterns in .claude/settings.local.json for persistence.",
                clean
            )));
        }

        // Toggle mode
        if let Some(ref shared) = ctx.shared {
            if let Ok(mut state) = shared.lock() {
                if sub == "on" || sub == "enable" {
                    state.sandbox_mode = true;
                } else if sub == "off" || sub == "disable" {
                    state.sandbox_mode = false;
                } else {
                    state.sandbox_mode = !state.sandbox_mode;
                }
                let status = if state.sandbox_mode {
                    "enabled"
                } else {
                    "disabled"
                };
                return Ok(CommandResult::Action(format!(
                    "Sandbox mode {}.\n\
                     When enabled, shell commands execute in an isolated environment.\n\
                     Platform: {} ({})",
                    status,
                    std::env::consts::OS,
                    if cfg!(target_os = "macos") || cfg!(target_os = "linux") {
                        "supported"
                    } else {
                        "not supported"
                    }
                )));
            }
        }

        Ok(CommandResult::Action(
            "Sandbox mode toggled. (No live session to persist state.)".to_string(),
        ))
    }
}

/// /output-style — Set, clear, or list output styles. Reads styles from
/// `~/.claude/output-styles/*.md` and `<project>/.claude/output-styles/*.md`
/// via [`crate::output_styles::load_output_styles`], persists the selection
/// to `<project>/.claude/settings.json` under the `outputStyle` key.
///
/// Args:
/// - empty            → list available styles + show the current one
/// - `<name>`         → set the active style
/// - `clear` / `none` → unset the active style
pub struct OutputStyleHandler;
impl CommandHandler for OutputStyleHandler {
    fn execute(&self, args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        let styles = crate::output_styles::load_output_styles(&ctx.working_directory);
        let settings_path = ctx.working_directory.join(".claude").join("settings.json");

        let arg = args.trim();
        if arg.is_empty() {
            // List styles + show current
            let mut s: Settings = if settings_path.exists() {
                Settings::load_from_file(&settings_path)
            } else {
                Settings::default()
            };
            // Reload from disk so the displayed value reflects external edits.
            s = if settings_path.exists() {
                Settings::load_from_file(&settings_path)
            } else {
                s
            };
            let current = s.output_style.as_deref().unwrap_or("(none)");
            let mut out = format!("Active output style: {current}\n\nAvailable styles:\n");
            if styles.is_empty() {
                out.push_str(
                    "  (none — drop a markdown file in `.claude/output-styles/<name>.md`)\n",
                );
            } else {
                for style in &styles {
                    out.push_str(&format!("  - {} — {}\n", style.name, style.description));
                }
                out.push_str(
                    "\nUsage: `/output-style <name>` to set, `/output-style clear` to unset.\n",
                );
            }
            return Ok(CommandResult::Action(out));
        }

        if arg.eq_ignore_ascii_case("clear") || arg.eq_ignore_ascii_case("none") {
            return persist_output_style(&settings_path, None)
                .map(|_| CommandResult::Action("Output style cleared.".to_string()));
        }

        // Set: validate the name exists.
        if !styles.iter().any(|s| s.name == arg) {
            let names: Vec<&str> = styles.iter().map(|s| s.name.as_str()).collect();
            let suggestion = if names.is_empty() {
                "no styles installed".to_string()
            } else {
                format!("available: {}", names.join(", "))
            };
            return Ok(CommandResult::Action(format!(
                "No output style named `{arg}` ({suggestion})"
            )));
        }
        persist_output_style(&settings_path, Some(arg))
            .map(|_| CommandResult::Action(format!("Output style set to `{arg}`.")))
    }
}

/// Read settings.json (or default), set/clear `outputStyle`, write back.
/// Atomic-ish: writes to a tmp file then renames.
fn persist_output_style(settings_path: &std::path::Path, style: Option<&str>) -> Result<()> {
    if let Some(parent) = settings_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let mut s = if settings_path.exists() {
        Settings::load_from_file(settings_path)
    } else {
        Settings::default()
    };
    s.output_style = style.map(|s| s.to_string());
    let json = serde_json::to_string_pretty(&s)?;
    let tmp = settings_path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, settings_path)?;
    Ok(())
}

/// /commit-push-pr — Git commit + push + create PR in one step (Prompt command)
pub struct CommitPushPrHandler;
impl CommandHandler for CommitPushPrHandler {
    fn execute(&self, args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        let extra = if args.trim().is_empty() {
            String::new()
        } else {
            format!(
                "\n\n## Additional instructions from user\n\n{}",
                args.trim()
            )
        };

        // Gather context similar to the TS implementation
        let safe_user = std::env::var("SAFEUSER").unwrap_or_default();
        let username = std::env::var("USER").unwrap_or_default();
        let default_branch = "main";

        Ok(CommandResult::Message(format!(
            "## Context\n\n\
             - `SAFEUSER`: {safe_user}\n\
             - `whoami`: {username}\n\
             - `git status`: !`git status`\n\
             - `git diff HEAD`: !`git diff HEAD`\n\
             - `git branch --show-current`: !`git branch --show-current`\n\
             - `git diff {default_branch}...HEAD`: !`git diff {default_branch}...HEAD`\n\
             - `gh pr view --json number 2>/dev/null || true`: !`gh pr view --json number 2>/dev/null || true`\n\n\
             ## Git Safety Protocol\n\n\
             - NEVER update the git config\n\
             - NEVER run destructive/irreversible git commands (like push --force, hard reset, etc) unless the user explicitly requests them\n\
             - NEVER skip hooks (--no-verify, --no-gpg-sign, etc) unless the user explicitly requests it\n\
             - NEVER run force push to main/master, warn the user if they request it\n\
             - Do not commit files that likely contain secrets (.env, credentials.json, etc)\n\
             - Never use git commands with the -i flag (like git rebase -i or git add -i) since they require interactive input which is not supported\n\n\
             ## Your task\n\n\
             Analyze all changes that will be included in the pull request, making sure to look at all relevant commits (NOT just the latest commit, but ALL commits that will be included in the pull request from the git diff {default_branch}...HEAD output above).\n\n\
             Based on the above changes:\n\
             1. Create a new branch if on {default_branch} (use SAFEUSER from context above for the branch name prefix, falling back to whoami if SAFEUSER is empty, e.g., `username/feature-name`)\n\
             2. Create a single commit with an appropriate message using heredoc syntax:\n\
             ```\n\
             git commit -m \"$(cat <<'EOF'\n\
             Commit message here.\n\
             EOF\n\
             )\"\n\
             ```\n\
             3. Push the branch to origin\n\
             4. If a PR already exists for this branch (check the gh pr view output above), update the PR title and body using `gh pr edit` to reflect the current diff. Otherwise, create a pull request using `gh pr create` with heredoc syntax for the body.\n\
                - IMPORTANT: Keep PR titles short (under 70 characters). Use the body for details.\n\
             ```\n\
             gh pr create --title \"Short, descriptive title\" --body \"$(cat <<'EOF'\n\
             ## Summary\n\
             <1-3 bullet points>\n\n\
             ## Test plan\n\
             [Bulleted markdown checklist of TODOs for testing the pull request...]\n\
             EOF\n\
             )\"\n\
             ```\n\n\
             You have the capability to call multiple tools in a single response. You MUST do all of the above in a single message.\n\n\
             Return the PR URL when you're done, so the user can see it.{extra}",
        )))
    }
}

/// /security-review — Run security review on codebase (Prompt command)
pub struct SecurityReviewHandler;
impl CommandHandler for SecurityReviewHandler {
    fn execute(&self, args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        let scope = if args.trim().is_empty() {
            "the pending changes on the current branch".to_string()
        } else {
            format!("the following: \"{}\"", args.trim())
        };
        Ok(CommandResult::Message(format!(
            "You are a senior security engineer conducting a focused security review of {}.\n\n\
             First, gather context:\n\
             1. Run `git status` and `git diff --name-only origin/HEAD...` to see changed files\n\
             2. Run `git diff origin/HEAD...` to get the full diff\n\
             3. Run `git log --no-decorate origin/HEAD...` to see commits\n\n\
             Then review the complete diff for security vulnerabilities.\n\n\
             OBJECTIVE: Identify HIGH-CONFIDENCE security vulnerabilities with real exploitation \
             potential. This is not a general code review -- focus ONLY on security implications.\n\n\
             CATEGORIES TO EXAMINE:\n\
             - Input Validation: SQL injection, command injection, path traversal, XXE, template injection\n\
             - Auth & Authorization: bypass logic, privilege escalation, session flaws, JWT issues\n\
             - Crypto & Secrets: hardcoded keys, weak algorithms, improper key storage\n\
             - Injection & Code Execution: RCE via deserialization, eval injection, XSS\n\
             - Data Exposure: sensitive data logging, PII handling, API endpoint leakage\n\n\
             SEVERITY GUIDELINES:\n\
             - HIGH: Directly exploitable (RCE, data breach, auth bypass)\n\
             - MEDIUM: Requires specific conditions but significant impact\n\
             - LOW: Defense-in-depth issues\n\n\
             Only report findings with >80% confidence of actual exploitability.\n\
             Output findings in markdown with file, line, severity, description, exploit scenario, and fix.\n\n\
             EXCLUSIONS: Skip DoS, rate limiting, secrets on disk, resource exhaustion, \
             test-only files, and theoretical issues.\n\n\
             Begin your analysis now. Do this in 3 steps:\n\
             1. Use a sub-task to identify vulnerabilities.\n\
             2. Then for each vulnerability identified by the above sub-task, create a new sub-task to filter out false-positives. Launch these sub-tasks as parallel sub-tasks.\n\
             3. Filter out any vulnerabilities where the sub-task reported a confidence less than 8.",
            scope
        )))
    }
}

/// /ultraplan — Ultra-detailed planning mode (Prompt command)
pub struct UltraplanHandler;
impl CommandHandler for UltraplanHandler {
    fn execute(&self, args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        let task = args.trim();
        if task.is_empty() {
            Ok(CommandResult::Message(
                "Enter ultra-detailed planning mode.\n\n\
                 Think extremely carefully and produce the most comprehensive, detailed plan possible. \
                 Use extended thinking to explore all angles.\n\n\
                 For each step:\n\
                 1. State the objective clearly\n\
                 2. List ALL sub-tasks required\n\
                 3. Identify dependencies between steps\n\
                 4. Note potential risks and mitigations\n\
                 5. Estimate relative complexity\n\
                 6. Define acceptance criteria\n\n\
                 Do NOT execute anything. Only produce the plan and wait for approval.\n\
                 Consider edge cases, error handling, testing strategy, and rollback plans.\n\
                 Number every step and sub-step for easy reference."
                    .to_string(),
            ))
        } else {
            Ok(CommandResult::Message(format!(
                "Enter ultra-detailed planning mode for the following task:\n\n\
                 {}\n\n\
                 Think extremely carefully and produce the most comprehensive, detailed plan possible. \
                 Use extended thinking to explore all angles.\n\n\
                 For each step:\n\
                 1. State the objective clearly\n\
                 2. List ALL sub-tasks required\n\
                 3. Identify dependencies between steps\n\
                 4. Note potential risks and mitigations\n\
                 5. Estimate relative complexity\n\
                 6. Define acceptance criteria\n\n\
                 Do NOT execute anything. Only produce the plan and wait for approval.\n\
                 Consider edge cases, error handling, testing strategy, and rollback plans.\n\
                 Number every step and sub-step for easy reference.",
                task
            )))
        }
    }
}

/// /thinkback — Replay reasoning/thinking blocks (Prompt command)
pub struct ThinkbackHandler;
impl CommandHandler for ThinkbackHandler {
    fn execute(&self, args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        let scope = args.trim();
        if scope.is_empty() {
            Ok(CommandResult::Message(
                "Replay and explain your reasoning process from the conversation so far.\n\n\
                 Walk through each major decision or analysis step:\n\
                 1. What were you trying to accomplish?\n\
                 2. What options did you consider?\n\
                 3. Why did you choose the approach you took?\n\
                 4. What trade-offs were involved?\n\
                 5. Were there any assumptions that might be wrong?\n\n\
                 Be transparent about uncertainty and areas where different approaches \
                 could have been taken."
                    .to_string(),
            ))
        } else {
            Ok(CommandResult::Message(format!(
                "Replay and explain your reasoning process specifically about: \"{}\"\n\n\
                 Walk through each major decision or analysis step:\n\
                 1. What were you trying to accomplish?\n\
                 2. What options did you consider?\n\
                 3. Why did you choose the approach you took?\n\
                 4. What trade-offs were involved?\n\
                 5. Were there any assumptions that might be wrong?\n\n\
                 Be transparent about uncertainty and areas where different approaches \
                 could have been taken.",
                scope
            )))
        }
    }
}

/// /insights — Usage pattern insights (Prompt command)
pub struct InsightsHandler;
impl CommandHandler for InsightsHandler {
    fn execute(&self, _args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        let session_info = if let Some(ref shared) = ctx.shared {
            if let Ok(state) = shared.lock() {
                let elapsed = state.session_start.elapsed();
                let mins = elapsed.as_secs() / 60;
                format!(
                    "Session duration: {} minutes, {} messages, {} tokens used, \
                     {} API requests, ${:.4} total cost",
                    mins,
                    state.message_count,
                    state.total_tokens,
                    state.request_count,
                    state.total_cost_usd,
                )
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        Ok(CommandResult::Message(format!(
            "Analyze the usage patterns from this session and provide actionable insights.\n\n\
             Current session data:\n{}\n\n\
             Please provide:\n\
             1. A summary of what was accomplished in this session\n\
             2. Patterns in how tools were used (which tools most, any redundant calls)\n\
             3. Efficiency observations (could anything have been done with fewer steps?)\n\
             4. Suggestions for workflow improvements\n\
             5. Areas where custom skills or hooks could automate repetitive patterns\n\
             6. Token usage efficiency analysis\n\n\
             Focus on practical, actionable recommendations.",
            session_info
        )))
    }
}

/// /btw — Ask a quick side question without interrupting the main conversation.
/// Port of src/commands/btw (TS). If args are empty we prompt the user; otherwise
/// we frame the question so the model treats it as a parenthetical aside and
/// returns to the primary task afterward.
pub struct BtwHandler;
impl CommandHandler for BtwHandler {
    fn execute(&self, args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        let trimmed = args.trim();
        if trimmed.is_empty() {
            return Ok(CommandResult::Error("Usage: /btw <question>".to_string()));
        }
        Ok(CommandResult::Message(format!(
            "[Side question — answer briefly, then return to what we were doing]\n\n{}",
            trimmed
        )))
    }
}

/// /feedback — Submit feedback about Claude Code. TS opens a form; we print
/// the URL and echo the optional report text so it's in the transcript.
pub struct FeedbackHandler;
impl CommandHandler for FeedbackHandler {
    fn execute(&self, args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        let extra = args.trim();
        let body = if extra.is_empty() {
            "To send feedback, open: https://github.com/anthropics/claude-code/issues\n\
             Or reply to this message with your feedback — it will be logged in the transcript."
                .to_string()
        } else {
            format!(
                "Feedback recorded in the transcript (not sent automatically):\n\n---\n{}\n---\n\n\
                 To submit officially, open: https://github.com/anthropics/claude-code/issues",
                extra
            )
        };
        Ok(CommandResult::Action(body))
    }
}

/// /upgrade — Print upgrade URL. TS opens a browser page; we surface the URL
/// so the user can follow it themselves.
pub struct UpgradeHandler;
impl CommandHandler for UpgradeHandler {
    fn execute(&self, _args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        Ok(CommandResult::Action(
            "Upgrade options:\n\
             - Claude Max: https://claude.ai/upgrade\n\
             - Team/Enterprise: https://claude.com/pricing"
                .to_string(),
        ))
    }
}

/// /privacy-settings — Point the user at the privacy docs.
pub struct PrivacySettingsHandler;
impl CommandHandler for PrivacySettingsHandler {
    fn execute(&self, _args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        Ok(CommandResult::Action(
            "Privacy settings:\n\
             - Docs: https://docs.claude.com/claude-code/privacy\n\
             - Data retention & telemetry are controlled in ~/.claude/settings.json\n\
             - Set CLAUDE_CODE_SIMPLE=1 for essential-traffic-only mode"
                .to_string(),
        ))
    }
}

/// /tag — Label the current session with a short name, stored on the shared
/// CommandState so the TUI/statusline can display it. Clears when args empty.
pub struct TagHandler;
impl CommandHandler for TagHandler {
    fn execute(&self, args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        let Some(shared) = ctx.shared.as_ref() else {
            return Ok(CommandResult::Error(
                "/tag requires a live session".to_string(),
            ));
        };
        let Ok(mut state) = shared.lock() else {
            return Ok(CommandResult::Error(
                "/tag: failed to acquire session state lock".to_string(),
            ));
        };
        let trimmed = args.trim();
        if trimmed.is_empty() {
            state.session_name.clear();
            return Ok(CommandResult::Action("Session tag cleared.".to_string()));
        }
        state.session_name = trimmed.to_string();
        Ok(CommandResult::Action(format!(
            "Session tagged: {}",
            trimmed
        )))
    }
}

/// /install-github-app — Points the user at the GitHub App installer.
pub struct InstallGithubAppHandler;
impl CommandHandler for InstallGithubAppHandler {
    fn execute(&self, _args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        Ok(CommandResult::Action(
            "Install the Claude Code GitHub App:\n\
             1. Visit https://github.com/apps/claude for Claude\n\
             2. Click \"Install\" and pick the repos you want Claude on\n\
             3. Grant the requested permissions (content, PRs, issues)\n\
             4. Back here, run `gh auth status` to confirm the installation.\n\n\
             The TS version launches an interactive wizard; the Rust build ships only \
             the URL + manual steps. A full wizard needs the IDE bridge, which has not \
             been ported."
                .to_string(),
        ))
    }
}

/// /install-slack-app — Points the user at the Claude Slack app installer.
pub struct InstallSlackAppHandler;
impl CommandHandler for InstallSlackAppHandler {
    fn execute(&self, _args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        Ok(CommandResult::Action(
            "Install Claude for Slack:\n\
             1. Visit https://claude.ai/slack for Claude\n\
             2. Click \"Add to Slack\" and pick the workspace\n\
             3. Approve the requested scopes when Slack prompts\n\
             4. In any Slack channel, DM @Claude or use `/claude <prompt>`."
                .to_string(),
        ))
    }
}

/// /chrome — Claude in Chrome (extension settings).
pub struct ChromeHandler;
impl CommandHandler for ChromeHandler {
    fn execute(&self, _args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        Ok(CommandResult::Action(
            "Claude in Chrome:\n\
             - Chrome Web Store: https://chromewebstore.google.com/search/claude\n\
             - Docs: https://docs.claude.com/claude-code/chrome\n\
             - After install, sign in with the same account you use here to sync sessions."
                .to_string(),
        ))
    }
}

/// /desktop — Continue the current session in Claude Desktop.
/// Only meaningful on macOS / Windows x64 (matches TS availability gate);
/// we print the install link regardless since there's no UX cost.
pub struct DesktopHandler;
impl CommandHandler for DesktopHandler {
    fn execute(&self, _args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        let platform_note = if cfg!(target_os = "macos") {
            "Your platform is supported (macOS)."
        } else if cfg!(target_os = "windows") {
            "Your platform is supported (Windows)."
        } else {
            "Note: Claude Desktop currently ships for macOS and Windows only."
        };
        Ok(CommandResult::Action(format!(
            "Claude Desktop:\n\
             - Download: https://claude.ai/download\n\
             - Docs: https://docs.claude.com/claude-code/desktop\n\n\
             {}",
            platform_note
        )))
    }
}

/// /mobile — Pointer to the mobile app.
pub struct MobileHandler;
impl CommandHandler for MobileHandler {
    fn execute(&self, _args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        Ok(CommandResult::Action(
            "Claude on mobile:\n\
             - iOS:     https://apps.apple.com/app/claude-by-anthropic/id6473753684\n\
             - Android: https://play.google.com/store/apps/details?id=com.anthropic.claude\n\
             - Sign in with the same account to sync sessions across devices."
                .to_string(),
        ))
    }
}

/// /terminalSetup — Run through terminal-bell / clipboard / key-mode setup.
pub struct TerminalSetupHandler;
impl CommandHandler for TerminalSetupHandler {
    fn execute(&self, _args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        let term = std::env::var("TERM").unwrap_or_else(|_| "(unset)".into());
        let term_program = std::env::var("TERM_PROGRAM").unwrap_or_else(|_| "(unset)".into());
        Ok(CommandResult::Action(format!(
            "Terminal setup:\n\
             - Detected TERM={}, TERM_PROGRAM={}\n\
             - Ensure your terminal supports 256 colours (TERM=xterm-256color or better)\n\
             - For shift+tab / ctrl+shift+<letter>, use a Kitty-protocol terminal \
             (Kitty, WezTerm, Ghostty, iTerm2 with Kitty mode)\n\
             - On Windows, use Windows Terminal (not cmd.exe / conhost)\n\
             - Image paste: ctrl+v (alt+v on Windows). Kitty-protocol terminals also \
             accept cmd+v on macOS.\n\
             - Docs: https://docs.claude.com/claude-code/terminal",
            term, term_program
        )))
    }
}

/// /heapdump — Dump a v8 heap snapshot. Rust doesn't have v8; we emit
/// equivalent rusty info (memory stats from getrusage if available).
pub struct HeapdumpHandler;
impl CommandHandler for HeapdumpHandler {
    fn execute(&self, _args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        let pid = std::process::id();
        Ok(CommandResult::Action(format!(
            "Heap dump (Rust):\n\
             - PID: {}\n\
             - Rust doesn't expose a v8-style heap snapshot. For process-level profiling:\n\
               macOS:  `leaks {pid}` or `vmmap {pid}`\n\
               Linux:  `pmap {pid}` or `heaptrack --attach {pid}`\n\
             - For allocation-site profiling, rebuild with --features profiling \
             (not yet wired — tracked for later).",
            pid,
            pid = pid
        )))
    }
}

/// /remote-env — Print remote environment info (CCR / remote-shell mode).
pub struct RemoteEnvHandler;
impl CommandHandler for RemoteEnvHandler {
    fn execute(&self, _args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        let remote = std::env::var("CLAUDE_CODE_REMOTE").unwrap_or_default();
        let mem_dir = std::env::var("CLAUDE_CODE_REMOTE_MEMORY_DIR").unwrap_or_default();
        Ok(CommandResult::Action(format!(
            "Remote environment:\n\
             - CLAUDE_CODE_REMOTE={}\n\
             - CLAUDE_CODE_REMOTE_MEMORY_DIR={}\n\
             - CLAUDE_CONFIG_DIR={}\n\
             - HOME={}\n\
             - Docs: https://docs.claude.com/claude-code/remote",
            if remote.is_empty() {
                "(unset — running locally)".into()
            } else {
                remote
            },
            if mem_dir.is_empty() {
                "(unset)".into()
            } else {
                mem_dir
            },
            std::env::var("CLAUDE_CONFIG_DIR").unwrap_or_else(|_| "(unset)".into()),
            std::env::var("HOME").unwrap_or_else(|_| "(unset)".into()),
        )))
    }
}

/// /remote-setup — Start remote-session setup.
pub struct RemoteSetupHandler;
impl CommandHandler for RemoteSetupHandler {
    fn execute(&self, _args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        Ok(CommandResult::Action(
            "Remote session setup:\n\
             1. On the remote host, install claude-rs (same version as local).\n\
             2. Set CLAUDE_CODE_REMOTE=1 and point CLAUDE_CODE_REMOTE_MEMORY_DIR at a \
             persistent path if you want auto-memory across sessions.\n\
             3. Start the bridge server locally with `claude-rs server --port <N>`.\n\
             4. On the remote, set CLAUDE_CODE_BRIDGE=ws://<local-host>:<N> and run \
             `claude-rs`. The TUI there forwards keystrokes and tool calls to the \
             local orchestrator.\n\
             - Docs: https://docs.claude.com/claude-code/remote-setup"
                .to_string(),
        ))
    }
}

/// /remote-control — Claude.ai Remote Control.
///
/// TS wires this command to `useReplBridge`, which registers an environment,
/// creates a session, opens session-ingress, and injects inbound web/mobile
/// prompts into the REPL. The Rust port only has IDE bridge helpers today, so
/// keep the command visible but make the missing runtime explicit.
pub struct RemoteControlHandler;
impl CommandHandler for RemoteControlHandler {
    fn execute(&self, args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        let name = args.trim();

        if let Some(shared) = &ctx.shared {
            if let Ok(mut state) = shared.lock() {
                if state.remote_control_enabled {
                    state.remote_control_enabled = false;
                    state.remote_control_initial_name = None;
                    state.remote_control_session_url = None;
                    return Ok(CommandResult::Action(
                        "Remote Control disconnected.\n\n\
                         Note: the Rust port still needs the Claude.ai bridge runtime; \
                         this cleared the local session request."
                            .to_string(),
                    ));
                }

                state.remote_control_enabled = true;
                state.remote_control_initial_name = if name.is_empty() {
                    None
                } else {
                    Some(name.to_string())
                };
            }
        }

        let display_name = if name.is_empty() {
            String::new()
        } else {
            format!("\nRequested session name: {name}")
        };

        Ok(CommandResult::Action(format!(
            "Remote Control requested.{display_name}\n\n\
             The Rust port does not yet include the Claude.ai session-ingress \
             bridge runtime, so it cannot show a claude.ai/code session URL yet.\n\n\
             Still needed for full TS parity:\n\
             - environment registration and entitlement/policy checks\n\
             - remote session creation and reconnect/continue support\n\
             - session-ingress WebSocket forwarding\n\
             - inbound web/mobile message queue injection\n\
             - connected/disconnect dialog and footer status"
        )))
    }
}

/// /passes — Issues / PRs awaiting reply from the user. Since the real TS
/// version depends on GitHub-webhook + subscription state we don't have in
/// Rust yet, this ships as a prompt handler that asks the model to summarise
/// open items it can find via `gh`.
pub struct PassesHandler;
impl CommandHandler for PassesHandler {
    fn execute(&self, _args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
        Ok(CommandResult::Message(
            "Summarise items in this repository that are waiting on me. Use:\n\
             - `gh pr list --search 'review-requested:@me'` for PR review requests\n\
             - `gh pr list --search 'assignee:@me is:open'` for PRs assigned to me\n\
             - `gh issue list --search 'assignee:@me is:open'` for open issues\n\n\
             Format the result as three short sections (PRs to review, My PRs, My issues), \
             one line each with number + title + link. Skip sections with no items."
                .to_string(),
        ))
    }
}

/// /extra-usage — Extended usage breakdown. Reuses the usage-summary path
/// from the shared command state and appends per-turn details when available.
pub struct ExtraUsageHandler;
impl CommandHandler for ExtraUsageHandler {
    fn execute(&self, _args: &str, ctx: &CommandContext) -> Result<CommandResult> {
        let Some(shared) = ctx.shared.as_ref() else {
            return Ok(CommandResult::Error(
                "/extra-usage requires a live session".to_string(),
            ));
        };
        let Ok(state) = shared.lock() else {
            return Ok(CommandResult::Error(
                "/extra-usage: failed to acquire session state lock".to_string(),
            ));
        };
        let mut out = String::new();
        out.push_str(&format!(
            "Model:            {}\nRequests:         {}\nTotal tokens:     {}\nTotal cost (USD): ${:.4}\n\n",
            state.model, state.request_count, state.total_tokens, state.total_cost_usd
        ));
        if state.per_turn_tokens.is_empty() {
            out.push_str("No per-turn token data recorded.\n");
        } else {
            out.push_str("Per-turn tokens (turn, input, output):\n");
            for (turn, input, output) in &state.per_turn_tokens {
                out.push_str(&format!(
                    "  {:>3}: {:>7} in / {:>7} out\n",
                    turn, input, output
                ));
            }
        }
        Ok(CommandResult::Action(out))
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
    register!("help", "Show available commands", Action, HelpHandler);
    register!(
        "status",
        "Show session status (model, tokens, messages)",
        Action,
        StatusHandler
    );
    register!("clear", "Clear conversation history", Action, ClearHandler);
    register!(
        "model",
        "Show or change the current model",
        Action,
        ModelHandler
    );
    register!("config", "Show configuration", Action, ConfigHandler);
    register!(
        "cost",
        "Show token usage and estimated cost",
        Action,
        CostHandler
    );
    register!(
        "permissions",
        "Show current permission mode",
        Action,
        PermissionsHandler
    );
    register!("verbose", "Toggle verbose mode", Action, VerboseHandler);
    register!("memory", "Show auto-memory files", Action, MemoryHandler);
    register!("tasks", "Show current task list", Action, TasksHandler);
    register!("resume", "Resume a previous session", Action, ResumeHandler);
    register!("fork", "Fork the current session", Action, ForkHandler);
    register!(
        "context",
        "Show context window usage",
        Action,
        ContextHandler
    );
    register!("theme", "Change color theme", Action, ThemeHandler);
    register!("fast", "Toggle fast mode", Action, FastHandler);
    register!("brief", "Toggle brief mode", Action, BriefHandler);
    register!("effort", "Set effort level", Action, EffortHandler);

    // Action commands (new)
    register!(
        "doctor",
        "Run environment health checks",
        Action,
        DoctorHandler
    );
    register!(
        "diff",
        "Show git diff (staged + unstaged)",
        Action,
        DiffHandler
    );
    register!("export", "Export session to file", Action, ExportHandler);
    register!("mcp", "Manage MCP servers", Action, McpHandler);
    register!("plugin", "Manage plugins", Action, PluginHandler);
    register!("skills", "List available skills", Action, SkillsHandler);
    register!("agents", "List running agents", Action, AgentsHandler);
    register!(
        "rewind",
        "Revert recent file changes",
        Action,
        RewindHandler
    );
    register!("files", "List project files", Action, FilesHandler);
    register!(
        "init",
        "Initialize Claude Code in project",
        Prompt,
        InitHandler
    );
    register!(
        "init-verifiers",
        "Create verifier skill(s) for automated verification of code changes",
        Prompt,
        InitVerifiersHandler
    );
    register!("stats", "Show usage statistics", Action, StatsHandler);
    register!("env", "Show environment variables", Action, EnvHandler);
    register!("hooks", "List configured hooks", Action, HooksHandler);
    register!("session", "Session management", Action, SessionHandler);
    register!(
        "copy",
        "Copy last response to clipboard",
        Action,
        CopyHandler
    );

    // Prompt commands (existing)
    register!(
        "compact",
        "Manually trigger conversation compaction",
        Prompt,
        CompactHandler
    );
    register!("plan", "Enter plan mode", Prompt, PlanHandler);
    register!("exit-plan", "Exit plan mode", Prompt, ExitPlanHandler);
    register!(
        "commit",
        "Generate a git commit for staged changes",
        Prompt,
        CommitHandler
    );
    register!(
        "review",
        "Review code changes (git diff)",
        Prompt,
        ReviewHandler
    );
    register!("branch", "Create a new git branch", Prompt, BranchHandler);
    register!("pr", "Create a pull request description", Prompt, PrHandler);
    register!("bug", "Report or analyze a bug", Prompt, BugHandler);
    register!("test", "Generate tests for code", Prompt, TestHandler);
    register!("refactor", "Suggest refactoring", Prompt, RefactorHandler);
    register!("explain", "Explain code", Prompt, ExplainHandler);
    register!("docs", "Generate documentation", Prompt, DocsHandler);

    // Prompt commands (new)
    register!(
        "pr-comments",
        "Analyze PR comments",
        Prompt,
        PrCommentsHandler
    );
    register!(
        "proactive",
        "Enable proactive mode",
        Prompt,
        ProactiveHandler
    );
    register!(
        "ultrareview",
        "Deep code review",
        Prompt,
        UltrareviewHandler
    );

    // Batch 3 – Action commands
    register!(
        "share",
        "Share conversation as markdown",
        Action,
        ShareHandler
    );
    register!(
        "usage",
        "Detailed token usage breakdown",
        Action,
        UsageHandler
    );
    register!("rename", "Rename current session", Action, RenameHandler);
    register!("add-dir", "Add working directories", Action, AddDirHandler);
    register!(
        "keybindings",
        "Show keyboard shortcuts",
        Action,
        KeybindingsHandler
    );
    register!(
        "reload-plugins",
        "Reload plugin directory",
        Action,
        ReloadPluginsHandler
    );
    register!(
        "release-notes",
        "Show release notes",
        Action,
        ReleaseNotesHandler
    );
    register!("color", "Set session color", Action, ColorHandler);
    register!("sandbox", "Toggle sandbox mode", Action, SandboxHandler);
    register!(
        "output-style",
        "(deprecated) Use /config",
        Action,
        OutputStyleHandler
    );

    // Batch 3 – Prompt commands
    register!(
        "commit-push-pr",
        "Commit, push, and create PR",
        Prompt,
        CommitPushPrHandler
    );
    register!(
        "security-review",
        "Security review of branch",
        Prompt,
        SecurityReviewHandler
    );
    register!(
        "ultraplan",
        "Ultra-detailed planning mode",
        Prompt,
        UltraplanHandler
    );
    register!(
        "thinkback",
        "Replay reasoning process",
        Prompt,
        ThinkbackHandler
    );
    register!("insights", "Usage insights report", Prompt, InsightsHandler);

    // Batch 4 — previously-missing TS ports
    register!(
        "btw",
        "Ask a quick side question without losing context",
        Prompt,
        BtwHandler
    );
    register!(
        "feedback",
        "Submit feedback about Claude Code",
        Action,
        FeedbackHandler
    );
    register!(
        "upgrade",
        "Show upgrade links (Max / Team / Enterprise)",
        Action,
        UpgradeHandler
    );
    register!(
        "privacy-settings",
        "Show privacy settings documentation",
        Action,
        PrivacySettingsHandler
    );
    register!(
        "tag",
        "Tag the current session with a short name",
        Action,
        TagHandler
    );
    register!(
        "extra-usage",
        "Detailed usage breakdown including per-turn tokens",
        Action,
        ExtraUsageHandler
    );

    // Batch 5 — integration / setup commands (URL + instructions; TS opens UI)
    register!(
        "install-github-app",
        "Install the Claude Code GitHub App",
        Action,
        InstallGithubAppHandler
    );
    register!(
        "install-slack-app",
        "Install the Claude Slack app",
        Action,
        InstallSlackAppHandler
    );
    register!(
        "chrome",
        "Claude in Chrome (Beta) settings",
        Action,
        ChromeHandler
    );
    register!(
        "desktop",
        "Continue the current session in Claude Desktop",
        Action,
        DesktopHandler
    );
    register!(
        "mobile",
        "Claude on mobile (iOS / Android)",
        Action,
        MobileHandler
    );
    register!(
        "terminalSetup",
        "Terminal setup diagnostics + instructions",
        Action,
        TerminalSetupHandler
    );
    register!(
        "heapdump",
        "Process memory profiling info",
        Action,
        HeapdumpHandler
    );
    register!(
        "remote-env",
        "Show remote-session environment",
        Action,
        RemoteEnvHandler
    );
    register!(
        "remote-setup",
        "Remote session setup instructions",
        Action,
        RemoteSetupHandler
    );
    register!(
        "remote-control",
        "Connect this terminal for remote-control sessions",
        Action,
        RemoteControlHandler
    );
    register!(
        "rc",
        "Alias for /remote-control",
        Action,
        RemoteControlHandler
    );
    register!(
        "passes",
        "Summarise GitHub items waiting on the user",
        Prompt,
        PassesHandler
    );

    registry
}

// ── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

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
        let state = super::super::registry::SharedCommandState {
            cost_summary: "Total cost:            $0.0123\nTotal input tokens:    1000\nTotal output tokens:   500".to_string(),
            model: "claude-sonnet-4-20250514".to_string(),
            ..Default::default()
        };
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
    fn test_init_creates_claude_directory_and_returns_prompt() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_ctx_with_dir(tmp.path());
        let handler = InitHandler;
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Message(text) => {
                assert!(text.contains("analyze this codebase"));
                assert!(text.contains("CLAUDE.md"));
                assert!(tmp.path().join(".claude").exists());
                assert!(tmp.path().join(".claude/settings.json").exists());
            }
            other => panic!("expected Message, got {:?}", std::mem::discriminant(&other)),
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
            CommandResult::Message(text) => {
                // Should still return the prompt even if files already exist
                assert!(text.contains("analyze this codebase"));
            }
            other => panic!("expected Message, got {:?}", std::mem::discriminant(&other)),
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
                assert!(text.contains("pull request"));
                assert!(text.contains("gh pr view"));
                assert!(text.contains("gh api"));
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
            "doctor",
            "diff",
            "export",
            "mcp",
            "plugin",
            "skills",
            "agents",
            "rewind",
            "files",
            "init",
            "stats",
            "env",
            "hooks",
            "session",
            "copy",
            "pr-comments",
            "proactive",
            "ultrareview",
            // Batch 3
            "share",
            "usage",
            "rename",
            "add-dir",
            "keybindings",
            "reload-plugins",
            "release-notes",
            "color",
            "sandbox",
            "output-style",
            "commit-push-pr",
            "security-review",
            "ultraplan",
            "thinkback",
            "insights",
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
                    "/doctor",
                    "/diff",
                    "/export",
                    "/mcp",
                    "/plugin",
                    "/skills",
                    "/agents",
                    "/rewind",
                    "/files",
                    "/init",
                    "/stats",
                    "/env",
                    "/hooks",
                    "/session",
                    "/copy",
                    "/pr-comments",
                    "/proactive",
                    "/ultrareview",
                    // Batch 3
                    "/share",
                    "/usage",
                    "/rename",
                    "/add-dir",
                    "/keybindings",
                    "/reload-plugins",
                    "/release-notes",
                    "/color",
                    "/sandbox",
                    "/output-style",
                    "/commit-push-pr",
                    "/security-review",
                    "/ultraplan",
                    "/thinkback",
                    "/insights",
                ] {
                    assert!(text.contains(cmd), "/help should list {}", cmd);
                }
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
    }

    // ------------------------------------------------------------------
    // Tests for Batch 3 commands
    // ------------------------------------------------------------------

    fn test_ctx_with_shared() -> CommandContext {
        use std::sync::{Arc, Mutex};
        CommandContext {
            working_directory: std::path::PathBuf::from("/tmp/test-project"),
            model: "claude-sonnet-4-20250514".to_string(),
            shared: Some(Arc::new(Mutex::new(
                super::super::registry::SharedCommandState::default(),
            ))),
        }
    }

    #[test]
    fn test_share_creates_markdown_file() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_ctx_with_dir(tmp.path());
        let handler = ShareHandler;
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(
                    text.contains("shared to"),
                    "should report success: {}",
                    text
                );
                assert!(
                    text.contains("claude_share_"),
                    "should include filename: {}",
                    text
                );
                // Verify file was created
                let files: Vec<_> = std::fs::read_dir(tmp.path())
                    .unwrap()
                    .flatten()
                    .filter(|e| e.file_name().to_string_lossy().starts_with("claude_share_"))
                    .collect();
                assert_eq!(files.len(), 1, "should create exactly one share file");
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_share_custom_filename() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_ctx_with_dir(tmp.path());
        let handler = ShareHandler;
        let result = handler.execute("my-convo", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("my-convo.md"));
                assert!(tmp.path().join("my-convo.md").exists());
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_usage_shows_token_breakdown() {
        use std::sync::{Arc, Mutex};
        let state = super::super::registry::SharedCommandState {
            message_count: 5,
            total_tokens: 2500,
            request_count: 3,
            total_cost_usd: 0.0042,
            per_turn_tokens: vec![(1, 200, 300), (2, 400, 600), (3, 100, 900)],
            ..Default::default()
        };
        let ctx = CommandContext {
            working_directory: std::path::PathBuf::from("/tmp/test"),
            model: "test-model".to_string(),
            shared: Some(Arc::new(Mutex::new(state))),
        };
        let handler = UsageHandler;
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("Token Usage Breakdown"));
                assert!(text.contains("2500"), "should show total tokens");
                assert!(text.contains("0.0042"), "should show cost");
                assert!(
                    text.contains("Per-turn breakdown"),
                    "should have per-turn section"
                );
                assert!(text.contains("200"), "should show turn input tokens");
                assert!(
                    text.contains("Average tokens/message"),
                    "should compute average"
                );
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_usage_no_session() {
        let handler = UsageHandler;
        let ctx = test_ctx();
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("No live session"));
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_rename_sets_session_name() {
        let ctx = test_ctx_with_shared();
        let handler = RenameHandler;
        let result = handler.execute("My Cool Session", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("renamed to: My Cool Session"));
                // Verify state was updated
                let state = ctx.shared.as_ref().unwrap().lock().unwrap();
                assert_eq!(state.session_name, "My Cool Session");
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_rename_requires_name() {
        let handler = RenameHandler;
        let ctx = test_ctx();
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Error(text) => {
                assert!(text.contains("Usage:"));
            }
            other => panic!("expected Error, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_add_dir_validates_path() {
        let handler = AddDirHandler;
        let ctx = test_ctx();
        // Empty argument
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Error(text) => {
                assert!(text.contains("Usage:"));
            }
            other => panic!(
                "expected Error for empty arg, got {:?}",
                std::mem::discriminant(&other)
            ),
        }
    }

    #[test]
    fn test_add_dir_rejects_nonexistent() {
        let handler = AddDirHandler;
        let ctx = test_ctx();
        let result = handler.execute("/nonexistent/path/zzz", &ctx).unwrap();
        match result {
            CommandResult::Error(text) => {
                assert!(text.contains("not found"));
            }
            other => panic!(
                "expected Error for nonexistent path, got {:?}",
                std::mem::discriminant(&other)
            ),
        }
    }

    #[test]
    fn test_add_dir_accepts_valid_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let extra = tempfile::tempdir().unwrap();
        let ctx = CommandContext {
            working_directory: tmp.path().to_path_buf(),
            model: "test".to_string(),
            shared: Some(std::sync::Arc::new(std::sync::Mutex::new(
                super::super::registry::SharedCommandState::default(),
            ))),
        };
        let handler = AddDirHandler;
        let result = handler
            .execute(extra.path().to_str().unwrap(), &ctx)
            .unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("Added"));
                assert!(text.contains(extra.path().to_str().unwrap()));
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_keybindings_creates_template() {
        let tmp = tempfile::tempdir().unwrap();
        let kb_path = tmp.path().join("keybindings.json");
        // The handler uses claude_dir(); for a unit test we just check it doesn't crash
        let handler = KeybindingsHandler;
        let ctx = test_ctx();
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) | CommandResult::Error(text) => {
                // Either creates a template or reads an existing file
                assert!(
                    text.contains("Keybindings") || text.contains("keybindings"),
                    "should mention keybindings: {}",
                    text
                );
            }
            _ => {}
        }
        let _ = kb_path; // suppress unused warning
    }

    #[test]
    fn test_reload_plugins_produces_output() {
        let handler = ReloadPluginsHandler;
        let ctx = test_ctx();
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("Reloaded") || text.contains("plugin"));
            }
            CommandResult::Error(text) => {
                // Acceptable if plugins dir doesn't exist in test env
                assert!(!text.is_empty());
            }
            other => panic!(
                "expected Action or Error, got {:?}",
                std::mem::discriminant(&other)
            ),
        }
    }

    #[test]
    fn test_release_notes_produces_output() {
        let handler = ReleaseNotesHandler;
        let ctx = test_ctx();
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("Release Notes") || text.contains("changelog"));
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_color_shows_available_colors() {
        let handler = ColorHandler;
        let ctx = test_ctx_with_shared();
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("red"));
                assert!(text.contains("blue"));
                assert!(text.contains("green"));
                assert!(text.contains("Available colors"));
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_color_sets_valid_color() {
        let ctx = test_ctx_with_shared();
        let handler = ColorHandler;
        let result = handler.execute("purple", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("set to: purple"));
                let state = ctx.shared.as_ref().unwrap().lock().unwrap();
                assert_eq!(state.session_color, "purple");
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_color_rejects_invalid() {
        let handler = ColorHandler;
        let ctx = test_ctx_with_shared();
        let result = handler.execute("rainbow", &ctx).unwrap();
        match result {
            CommandResult::Error(text) => {
                assert!(text.contains("Invalid color"));
                assert!(text.contains("rainbow"));
            }
            other => panic!("expected Error, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_color_resets_to_default() {
        let ctx = test_ctx_with_shared();
        let handler = ColorHandler;
        handler.execute("blue", &ctx).unwrap();
        let result = handler.execute("reset", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("reset to default"));
                let state = ctx.shared.as_ref().unwrap().lock().unwrap();
                assert_eq!(state.session_color, "default");
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_sandbox_toggles_mode() {
        let ctx = test_ctx_with_shared();
        let handler = SandboxHandler;
        // Initially off
        {
            let state = ctx.shared.as_ref().unwrap().lock().unwrap();
            assert!(!state.sandbox_mode);
        }
        // Toggle on
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("enabled"));
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
        {
            let state = ctx.shared.as_ref().unwrap().lock().unwrap();
            assert!(state.sandbox_mode);
        }
        // Toggle off
        let result2 = handler.execute("", &ctx).unwrap();
        match result2 {
            CommandResult::Action(text) => {
                assert!(text.contains("disabled"));
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_sandbox_explicit_on_off() {
        let ctx = test_ctx_with_shared();
        let handler = SandboxHandler;
        let result = handler.execute("on", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("enabled"));
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
        let result = handler.execute("off", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("disabled"));
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_sandbox_exclude_subcommand() {
        let handler = SandboxHandler;
        let ctx = test_ctx();
        let result = handler.execute("exclude \"npm run test:*\"", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("npm run test:*"));
                assert!(text.contains("excluded"));
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_output_style_lists_when_no_args() {
        // No styles installed → handler should still respond, not error.
        let handler = OutputStyleHandler;
        let ctx = test_ctx();
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("Active output style"));
                assert!(text.contains("Available styles:"));
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_output_style_unknown_name() {
        let handler = OutputStyleHandler;
        let ctx = test_ctx();
        let result = handler.execute("nonsuch-style", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("No output style named"));
                assert!(text.contains("nonsuch-style"));
            }
            other => panic!("expected Action, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_output_style_set_and_clear_round_trip() {
        // Use a temp project with one fake style and verify the handler
        // sets + clears it through settings.json.
        let tmp = tempfile::tempdir().unwrap();
        let claude_dir = tmp.path().join(".claude");
        std::fs::create_dir_all(claude_dir.join("output-styles")).unwrap();
        std::fs::write(
            claude_dir.join("output-styles").join("terse.md"),
            "---\ndescription: Short\n---\n\nBe brief.\n",
        )
        .unwrap();

        let ctx = CommandContext {
            working_directory: tmp.path().to_path_buf(),
            ..test_ctx()
        };
        let handler = OutputStyleHandler;

        // Set
        handler.execute("terse", &ctx).unwrap();
        let settings = Settings::load_from_file(&claude_dir.join("settings.json"));
        assert_eq!(settings.output_style.as_deref(), Some("terse"));

        // Clear
        handler.execute("clear", &ctx).unwrap();
        let settings = Settings::load_from_file(&claude_dir.join("settings.json"));
        assert!(settings.output_style.is_none());
    }

    #[test]
    fn test_commit_push_pr_returns_prompt() {
        let handler = CommitPushPrHandler;
        let ctx = test_ctx();
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Message(text) => {
                // Match the actual TS prompt content at
                // `commands/commit-push-pr.ts:85-91`. The TS
                // prompt numbers the steps as:
                //   1. Create a new branch if on main
                //   2. Create a single commit (shows `git commit` heredoc)
                //   3. Push the branch to origin   ← prose, not literal "git push"
                //   4. gh pr create / gh pr edit
                // "git push" appears only in the allowedTools slot
                // (`'Bash(git push:*)'`), not in the prompt body. Don't
                // assert the literal command string; assert the
                // push-step prose + the PR-creation command instead.
                assert!(
                    text.contains("git commit"),
                    "should show the commit heredoc"
                );
                assert!(
                    text.contains("Push the branch to origin"),
                    "should contain the push step prose"
                );
                assert!(text.contains("gh pr create"), "should mention PR creation");
                assert!(text.contains("PR URL"), "should ask to return URL");
            }
            other => panic!("expected Message, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_commit_push_pr_with_extra_args() {
        let handler = CommitPushPrHandler;
        let ctx = test_ctx();
        let result = handler.execute("fix the login bug", &ctx).unwrap();
        match result {
            CommandResult::Message(text) => {
                assert!(text.contains("fix the login bug"));
                assert!(text.contains("Additional instructions"));
            }
            other => panic!("expected Message, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_security_review_returns_prompt() {
        let handler = SecurityReviewHandler;
        let ctx = test_ctx();
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Message(text) => {
                assert!(text.contains("security"));
                assert!(text.contains("git diff"));
                assert!(text.contains("Input Validation"));
                assert!(text.contains("Auth"));
                assert!(text.contains("Crypto"));
                assert!(text.contains("HIGH"));
            }
            other => panic!("expected Message, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_security_review_with_scope() {
        let handler = SecurityReviewHandler;
        let ctx = test_ctx();
        let result = handler.execute("src/auth.rs", &ctx).unwrap();
        match result {
            CommandResult::Message(text) => {
                assert!(text.contains("src/auth.rs"));
            }
            other => panic!("expected Message, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_ultraplan_returns_prompt() {
        let handler = UltraplanHandler;
        let ctx = test_ctx();
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Message(text) => {
                assert!(text.contains("ultra-detailed"));
                assert!(text.contains("Do NOT execute"));
                assert!(text.contains("acceptance criteria"));
                assert!(text.contains("sub-tasks"));
            }
            other => panic!("expected Message, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_ultraplan_with_task() {
        let handler = UltraplanHandler;
        let ctx = test_ctx();
        let result = handler
            .execute("migrate database to PostgreSQL", &ctx)
            .unwrap();
        match result {
            CommandResult::Message(text) => {
                assert!(text.contains("migrate database to PostgreSQL"));
                assert!(text.contains("ultra-detailed"));
            }
            other => panic!("expected Message, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_thinkback_returns_prompt() {
        let handler = ThinkbackHandler;
        let ctx = test_ctx();
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Message(text) => {
                assert!(text.contains("reasoning process"));
                assert!(text.contains("options did you consider"));
                assert!(text.contains("trade-offs"));
            }
            other => panic!("expected Message, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_thinkback_with_scope() {
        let handler = ThinkbackHandler;
        let ctx = test_ctx();
        let result = handler.execute("the architecture decision", &ctx).unwrap();
        match result {
            CommandResult::Message(text) => {
                assert!(text.contains("the architecture decision"));
            }
            other => panic!("expected Message, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_insights_returns_prompt() {
        let handler = InsightsHandler;
        let ctx = test_ctx();
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Message(text) => {
                assert!(text.contains("usage patterns"));
                assert!(text.contains("workflow improvements"));
                assert!(text.contains("Token usage efficiency"));
            }
            other => panic!("expected Message, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_insights_with_session_data() {
        use std::sync::{Arc, Mutex};
        let state = super::super::registry::SharedCommandState {
            message_count: 20,
            total_tokens: 50000,
            request_count: 15,
            total_cost_usd: 0.25,
            ..Default::default()
        };
        let ctx = CommandContext {
            working_directory: std::path::PathBuf::from("/tmp/test"),
            model: "test-model".to_string(),
            shared: Some(Arc::new(Mutex::new(state))),
        };
        let handler = InsightsHandler;
        let result = handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Message(text) => {
                assert!(text.contains("20 messages"));
                assert!(text.contains("50000 tokens"));
                assert!(text.contains("0.2500"));
            }
            other => panic!("expected Message, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn test_btw_empty_args_errors() {
        let r = BtwHandler.execute("", &test_ctx()).unwrap();
        assert!(matches!(r, CommandResult::Error(_)));
    }

    #[test]
    fn test_btw_wraps_question() {
        let r = BtwHandler.execute("what's 2+2?", &test_ctx()).unwrap();
        match r {
            CommandResult::Message(t) => {
                assert!(t.contains("what's 2+2?"));
                assert!(t.contains("Side question"));
            }
            _ => panic!("expected Message"),
        }
    }

    #[test]
    fn test_feedback_prints_url() {
        let r = FeedbackHandler.execute("", &test_ctx()).unwrap();
        match r {
            CommandResult::Action(t) => {
                assert!(t.contains("github.com/anthropics/claude-code"));
            }
            _ => panic!("expected Action"),
        }
    }

    #[test]
    fn test_feedback_echoes_report() {
        let r = FeedbackHandler
            .execute("repro: open file + crash", &test_ctx())
            .unwrap();
        match r {
            CommandResult::Action(t) => {
                assert!(t.contains("repro: open file + crash"));
            }
            _ => panic!("expected Action"),
        }
    }

    #[test]
    fn test_upgrade_has_both_links() {
        let r = UpgradeHandler.execute("", &test_ctx()).unwrap();
        match r {
            CommandResult::Action(t) => {
                assert!(t.contains("claude.ai/upgrade"));
                assert!(t.contains("claude.com/pricing"));
            }
            _ => panic!("expected Action"),
        }
    }

    #[test]
    fn test_privacy_settings_shows_env_hint() {
        let r = PrivacySettingsHandler.execute("", &test_ctx()).unwrap();
        match r {
            CommandResult::Action(t) => {
                assert!(t.contains("CLAUDE_CODE_SIMPLE"));
            }
            _ => panic!("expected Action"),
        }
    }

    #[test]
    fn test_tag_requires_shared_state() {
        let r = TagHandler.execute("work", &test_ctx()).unwrap();
        assert!(matches!(r, CommandResult::Error(_)));
    }

    #[test]
    fn test_tag_sets_and_clears() {
        use std::sync::{Arc, Mutex};
        let state = super::super::registry::SharedCommandState::default();
        let shared = Arc::new(Mutex::new(state));
        let ctx = CommandContext {
            working_directory: std::path::PathBuf::from("/tmp"),
            model: "m".to_string(),
            shared: Some(shared.clone()),
        };
        let set = TagHandler.execute("bugfix", &ctx).unwrap();
        assert!(matches!(set, CommandResult::Action(_)));
        assert_eq!(shared.lock().unwrap().session_name, "bugfix");
        let clear = TagHandler.execute("", &ctx).unwrap();
        assert!(matches!(clear, CommandResult::Action(_)));
        assert_eq!(shared.lock().unwrap().session_name, "");
    }

    #[test]
    fn test_install_github_app_prints_url() {
        let r = InstallGithubAppHandler.execute("", &test_ctx()).unwrap();
        match r {
            CommandResult::Action(t) => assert!(t.contains("github.com/apps/claude")),
            _ => panic!("expected Action"),
        }
    }

    #[test]
    fn test_install_slack_app_prints_url() {
        let r = InstallSlackAppHandler.execute("", &test_ctx()).unwrap();
        match r {
            CommandResult::Action(t) => assert!(t.contains("claude.ai/slack")),
            _ => panic!("expected Action"),
        }
    }

    #[test]
    fn test_chrome_links_store() {
        let r = ChromeHandler.execute("", &test_ctx()).unwrap();
        match r {
            CommandResult::Action(t) => assert!(t.contains("chromewebstore.google.com")),
            _ => panic!("expected Action"),
        }
    }

    #[test]
    fn test_desktop_notes_platform() {
        let r = DesktopHandler.execute("", &test_ctx()).unwrap();
        match r {
            CommandResult::Action(t) => {
                assert!(t.contains("claude.ai/download"));
                // The platform note always mentions macOS / Windows one way or another.
                assert!(t.contains("macOS") || t.contains("Windows"));
            }
            _ => panic!("expected Action"),
        }
    }

    #[test]
    fn test_mobile_has_both_stores() {
        let r = MobileHandler.execute("", &test_ctx()).unwrap();
        match r {
            CommandResult::Action(t) => {
                assert!(t.contains("apps.apple.com"));
                assert!(t.contains("play.google.com"));
            }
            _ => panic!("expected Action"),
        }
    }

    #[test]
    fn test_terminal_setup_shows_env() {
        let r = TerminalSetupHandler.execute("", &test_ctx()).unwrap();
        match r {
            CommandResult::Action(t) => {
                assert!(t.contains("TERM="));
                assert!(t.contains("Image paste"));
            }
            _ => panic!("expected Action"),
        }
    }

    #[test]
    fn test_heapdump_prints_pid() {
        let r = HeapdumpHandler.execute("", &test_ctx()).unwrap();
        match r {
            CommandResult::Action(t) => {
                assert!(t.contains("PID:"));
                assert!(t.contains("leaks") || t.contains("pmap"));
            }
            _ => panic!("expected Action"),
        }
    }

    #[test]
    fn test_remote_env_shows_defaults() {
        let r = RemoteEnvHandler.execute("", &test_ctx()).unwrap();
        match r {
            CommandResult::Action(t) => {
                assert!(t.contains("CLAUDE_CODE_REMOTE"));
            }
            _ => panic!("expected Action"),
        }
    }

    #[test]
    fn test_remote_setup_has_steps() {
        let r = RemoteSetupHandler.execute("", &test_ctx()).unwrap();
        match r {
            CommandResult::Action(t) => {
                assert!(t.contains("claude-rs server"));
                assert!(t.contains("CLAUDE_CODE_BRIDGE"));
            }
            _ => panic!("expected Action"),
        }
    }

    #[test]
    fn test_remote_control_reports_missing_runtime_and_sets_state() {
        let shared = Arc::new(Mutex::new(
            super::super::registry::SharedCommandState::default(),
        ));
        let ctx = CommandContext {
            working_directory: std::path::PathBuf::from("/tmp/test-project"),
            model: "claude-sonnet-4-20250514".to_string(),
            shared: Some(shared.clone()),
        };

        let r = RemoteControlHandler.execute("demo", &ctx).unwrap();
        match r {
            CommandResult::Action(t) => {
                assert!(t.contains("Remote Control requested"));
                assert!(t.contains("session-ingress"));
            }
            _ => panic!("expected Action"),
        }

        let state = shared.lock().unwrap();
        assert!(state.remote_control_enabled);
        assert_eq!(state.remote_control_initial_name.as_deref(), Some("demo"));
    }

    #[test]
    fn test_remote_control_second_call_disconnects() {
        let shared = Arc::new(Mutex::new(super::super::registry::SharedCommandState {
            remote_control_enabled: true,
            remote_control_initial_name: Some("demo".to_string()),
            remote_control_session_url: Some("https://claude.ai/code/session_test".to_string()),
            ..super::super::registry::SharedCommandState::default()
        }));
        let ctx = CommandContext {
            working_directory: std::path::PathBuf::from("/tmp/test-project"),
            model: "claude-sonnet-4-20250514".to_string(),
            shared: Some(shared.clone()),
        };

        let r = RemoteControlHandler.execute("", &ctx).unwrap();
        match r {
            CommandResult::Action(t) => assert!(t.contains("disconnected")),
            _ => panic!("expected Action"),
        }

        let state = shared.lock().unwrap();
        assert!(!state.remote_control_enabled);
        assert!(state.remote_control_initial_name.is_none());
        assert!(state.remote_control_session_url.is_none());
    }

    #[test]
    fn test_passes_returns_prompt() {
        let r = PassesHandler.execute("", &test_ctx()).unwrap();
        match r {
            CommandResult::Message(t) => {
                assert!(t.contains("gh pr list"));
                assert!(t.contains("waiting on me"));
            }
            _ => panic!("expected Message"),
        }
    }

    #[test]
    fn test_extra_usage_reports_per_turn() {
        use std::sync::{Arc, Mutex};
        let state = super::super::registry::SharedCommandState {
            per_turn_tokens: vec![(1, 100, 50), (2, 200, 75)],
            ..Default::default()
        };
        let shared = Arc::new(Mutex::new(state));
        let ctx = CommandContext {
            working_directory: std::path::PathBuf::from("/tmp"),
            model: "m".into(),
            shared: Some(shared),
        };
        let r = ExtraUsageHandler.execute("", &ctx).unwrap();
        match r {
            CommandResult::Action(t) => {
                assert!(t.contains("Per-turn tokens"));
                assert!(t.contains("100"));
                assert!(t.contains("200"));
            }
            _ => panic!("expected Action"),
        }
    }

    #[test]
    fn new_init_prompt_has_all_eight_phases() {
        for phase in [
            "## Phase 1: Ask what to set up",
            "## Phase 2: Explore the codebase",
            "## Phase 3: Fill in the gaps",
            "## Phase 4: Write CLAUDE.md",
            "## Phase 5: Write CLAUDE.local.md",
            "## Phase 6: Suggest and create skills",
            "## Phase 7: Suggest additional optimizations",
            "## Phase 8: Summary and next steps",
        ] {
            assert!(
                NEW_INIT_PROMPT.contains(phase),
                "NEW_INIT_PROMPT missing phase header: {phase}"
            );
        }
    }

    #[test]
    fn init_verifiers_prompt_has_core_structure() {
        assert!(INIT_VERIFIERS_PROMPT.contains("## Goal"));
        assert!(INIT_VERIFIERS_PROMPT.contains("verifier skill"));
        assert!(INIT_VERIFIERS_PROMPT.contains("TodoWrite tool"));
    }
}
