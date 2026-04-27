use anyhow::Result;
use clap::{Parser, ValueEnum};
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Parser)]
#[command(
    name = "claude-rs",
    about = "Claude Code - AI coding assistant (Rust port)",
    version
)]
pub struct Cli {
    /// Initial prompt (non-interactive mode)
    pub prompt: Option<String>,

    /// Print response and exit (TS-compatible alias for non-interactive mode)
    #[arg(short = 'p', long = "print")]
    pub print: bool,

    /// Output format for non-interactive mode
    #[arg(long = "output-format", value_enum, default_value_t = OutputFormat::Text)]
    pub output_format: OutputFormat,

    /// Model to use
    #[arg(short, long)]
    pub model: Option<String>,

    /// Effort level for the current session
    #[arg(long = "effort")]
    pub effort: Option<String>,

    /// Verbose output
    #[arg(short, long)]
    pub verbose: bool,

    /// Skip all permission checks (dangerous)
    #[arg(long)]
    pub dangerously_skip_permissions: bool,

    /// Permission mode to use for the session
    #[arg(long = "permission-mode")]
    pub permission_mode: Option<String>,

    /// Comma or space-separated list of tool permission rules to allow
    #[arg(
        long = "allowedTools",
        alias = "allowed-tools",
        num_args = 1..,
        value_delimiter = ','
    )]
    pub allowed_tools: Vec<String>,

    /// Comma or space-separated list of tool permission rules to deny
    #[arg(
        long = "disallowedTools",
        alias = "disallowed-tools",
        num_args = 1..,
        value_delimiter = ','
    )]
    pub disallowed_tools: Vec<String>,

    /// Specify available tools ("default", "", or comma/space-separated tool names)
    #[arg(long = "tools", num_args = 1.., value_delimiter = ',')]
    pub tools: Vec<String>,

    /// Additional directories to allow tool access to
    #[arg(long = "add-dir", num_args = 1..)]
    pub add_dirs: Vec<String>,

    /// MCP tool to use for permission prompts in print mode
    #[arg(long = "permission-prompt-tool", hide = true)]
    pub permission_prompt_tool: Option<String>,

    /// Working directory
    #[arg(short = 'C', long = "cd")]
    pub working_dir: Option<PathBuf>,

    /// Resume session by ID
    #[arg(long)]
    pub resume: Option<String>,

    /// Use a specific session ID
    #[arg(long = "session-id")]
    pub session_id: Option<String>,

    /// Disable session persistence (accepted for TS CLI parity; non-interactive Rust mode does not persist sessions)
    #[arg(long = "no-session-persistence")]
    pub no_session_persistence: bool,

    /// Max conversation turns (non-interactive)
    #[arg(long)]
    pub max_turns: Option<u32>,

    /// Append text to system prompt
    #[arg(long)]
    pub append_system_prompt: Option<String>,

    #[command(subcommand)]
    pub command: Option<SubCommand>,
}

#[derive(Clone, Debug, ValueEnum, PartialEq, Eq)]
pub enum OutputFormat {
    Text,
    Json,
    StreamJson,
}

#[derive(clap::Subcommand)]
pub enum SubCommand {
    /// Authenticate with Anthropic
    Login {
        /// Pre-populate email on the login form
        #[arg(long)]
        email: Option<String>,
        /// Force SSO login method
        #[arg(long)]
        sso: bool,
        /// Use Console auth flow (API key creation)
        #[arg(long)]
        console: bool,
        /// Use Claude.ai auth flow (default)
        #[arg(long)]
        claudeai: bool,
    },
    /// Remove stored credentials
    Logout,
    /// Show current configuration
    Config,
    /// Start the IDE bridge server
    Server {
        #[arg(long)]
        port: Option<u16>,
    },
    /// Connect your local environment for remote-control sessions via claude.ai/code
    #[command(alias = "rc", aliases = ["remote", "sync", "bridge"])]
    RemoteControl {
        /// Optional session name
        name: Option<String>,
    },
}

/// Resolve short model names to full API model IDs.
fn normalize_model_name(name: &str) -> String {
    let trimmed = name.trim();
    let lower = trimmed.to_lowercase();
    let has_1m = lower.ends_with("[1m]");
    let base = if has_1m {
        lower.trim_end_matches("[1m]").trim()
    } else {
        lower.as_str()
    };
    let suffix = if has_1m { "[1m]" } else { "" };

    match base {
        "opus" => format!("{}{}", default_opus_model(), suffix),
        "sonnet" => format!("{}{}", default_sonnet_model(), suffix),
        "haiku" => format!("{}{}", default_haiku_model(), suffix),
        "best" => default_opus_model(),
        "opusplan" => format!("{}{}", default_sonnet_model(), suffix),
        _ => trimmed.into(),
    }
}

fn default_opus_model() -> String {
    std::env::var("ANTHROPIC_DEFAULT_OPUS_MODEL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "claude-opus-4-7".into())
}

fn default_sonnet_model() -> String {
    std::env::var("ANTHROPIC_DEFAULT_SONNET_MODEL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "claude-sonnet-4-6".into())
}

fn default_haiku_model() -> String {
    std::env::var("ANTHROPIC_DEFAULT_HAIKU_MODEL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "claude-haiku-4-5".into())
}

async fn default_main_loop_model_setting() -> String {
    if claude_core::user_type::is_ant() {
        return format!("{}[1m]", default_opus_model());
    }

    let tokens = claude_core::auth::storage::load_tokens()
        .await
        .ok()
        .flatten();
    let subscription_type = tokens
        .as_ref()
        .and_then(|tokens| tokens.subscription_type.as_deref());
    let rate_limit_tier = tokens
        .as_ref()
        .and_then(|tokens| tokens.rate_limit_tier.as_deref());

    let is_max = subscription_type == Some("max");
    let is_team_premium =
        subscription_type == Some("team") && rate_limit_tier == Some("default_claude_max_5x");

    if is_max || is_team_premium {
        format!("{}[1m]", default_opus_model())
    } else {
        default_sonnet_model()
    }
}

fn resolve_max_output_tokens(
    model: &str,
    settings: &claude_core::config::settings::Settings,
) -> u64 {
    if let Ok(value) = std::env::var("CLAUDE_CODE_MAX_OUTPUT_TOKENS") {
        if let Ok(parsed) = value.trim().parse::<u64>() {
            return parsed;
        }
    }

    if let Some(max_tokens) = settings.max_tokens {
        return u64::from(max_tokens);
    }

    claude_core::api::client::get_max_output_tokens_for_model(model)
}

fn is_env_defined_falsy(name: &str) -> bool {
    std::env::var(name)
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            matches!(value.as_str(), "0" | "false" | "no" | "off")
        })
        .unwrap_or(false)
}

fn normalize_name_for_mcp(name: &str) -> String {
    let mut normalized: String = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect();

    if name.starts_with("claude.ai ") {
        let mut collapsed = String::with_capacity(normalized.len());
        let mut previous_underscore = false;
        for ch in normalized.chars() {
            if ch == '_' {
                if !previous_underscore {
                    collapsed.push(ch);
                }
                previous_underscore = true;
            } else {
                collapsed.push(ch);
                previous_underscore = false;
            }
        }
        normalized = collapsed.trim_matches('_').to_string();
    }

    normalized
}

fn mcp_server_signature(config: &claude_core::mcp::types::ScopedMcpServerConfig) -> Option<String> {
    use claude_core::mcp::types::McpServerConfig;

    match &config.config {
        McpServerConfig::Stdio(stdio) => {
            let mut command = Vec::with_capacity(1 + stdio.args.len());
            command.push(stdio.command.clone());
            command.extend(stdio.args.clone());
            serde_json::to_string(&command)
                .ok()
                .map(|value| format!("stdio:{}", value))
        }
        McpServerConfig::Sse(sse) | McpServerConfig::SseIde(sse) => {
            Some(format!("url:{}", unwrap_ccr_proxy_url(&sse.url)))
        }
        McpServerConfig::Http(http) => Some(format!("url:{}", unwrap_ccr_proxy_url(&http.url))),
        McpServerConfig::Ws(ws) | McpServerConfig::WsIde(ws) => {
            Some(format!("url:{}", unwrap_ccr_proxy_url(&ws.url)))
        }
    }
}

fn unwrap_ccr_proxy_url(url: &str) -> String {
    const MARKERS: [&str; 2] = ["/v2/session_ingress/shttp/mcp/", "/v2/ccr-sessions/"];
    if !MARKERS.iter().any(|marker| url.contains(marker)) {
        return url.to_string();
    }

    let Some((_, query)) = url.split_once('?') else {
        return url.to_string();
    };
    for pair in query.split('&') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        if key == "mcp_url" {
            return value.replace("%3A", ":").replace("%2F", "/");
        }
    }
    url.to_string()
}

fn split_tool_args(values: &[String]) -> Vec<String> {
    values
        .iter()
        .flat_map(|value| value.split_whitespace())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn filter_registry_by_cli_tools(registry: &mut claude_tools::ToolRegistry, values: &[String]) {
    if values.is_empty() {
        return;
    }

    let requested = split_tool_args(values);
    if requested.iter().any(|value| value == "default") {
        return;
    }

    if requested.is_empty() {
        for name in registry
            .all()
            .iter()
            .map(|tool| tool.name().to_string())
            .collect::<Vec<_>>()
        {
            registry.remove(&name);
        }
        return;
    }

    let keep = requested
        .into_iter()
        .filter_map(|name| registry.get(&name).map(|tool| tool.name().to_string()))
        .collect::<std::collections::HashSet<_>>();

    for name in registry
        .all()
        .iter()
        .map(|tool| tool.name().to_string())
        .collect::<Vec<_>>()
    {
        if !keep.contains(&name) {
            registry.remove(&name);
        }
    }
}

fn base_tool_denials_from_cli_tools(
    base_tools_cli: &[String],
    all_tool_names: &[String],
) -> Vec<String> {
    use claude_core::permissions::{normalize_legacy_tool_name, parse_tool_list_from_cli};

    if base_tools_cli.is_empty() {
        return Vec::new();
    }

    let joined = base_tools_cli.join(" ");
    let trimmed = joined.trim();
    if trimmed.is_empty() || trimmed == "default" {
        return Vec::new();
    }

    let base_tools = parse_tool_list_from_cli(base_tools_cli)
        .into_iter()
        .map(|name| normalize_legacy_tool_name(&name))
        .collect::<std::collections::HashSet<_>>();
    if base_tools.is_empty() {
        return all_tool_names.to_vec();
    }

    all_tool_names
        .iter()
        .filter(|name| !base_tools.contains(*name))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use claude_core::permissions::types::{PermissionBehavior, PermissionRuleSource};
    use claude_core::permissions::{
        parse_permission_rules_from_settings_value,
        permission_additional_directories_from_settings_value,
    };

    #[test]
    fn permission_settings_parse_rules_and_directories() {
        let value = serde_json::json!({
            "permissions": {
                "allow": ["Bash(git status)"],
                "deny": ["Write"],
                "ask": ["Edit"],
                "additionalDirectories": ["/tmp/work", 42, "/tmp/other"]
            }
        });

        let rules =
            parse_permission_rules_from_settings_value(&value, PermissionRuleSource::LocalSettings);
        assert_eq!(rules.len(), 3);
        assert!(rules.iter().any(|rule| {
            rule.source == PermissionRuleSource::LocalSettings
                && rule.rule_behavior == PermissionBehavior::Allow
                && rule.rule_value.to_rule_string() == "Bash(git status)"
        }));
        assert!(rules.iter().any(|rule| {
            rule.rule_behavior == PermissionBehavior::Deny
                && rule.rule_value.to_rule_string() == "Write"
        }));
        assert!(rules.iter().any(|rule| {
            rule.rule_behavior == PermissionBehavior::Ask
                && rule.rule_value.to_rule_string() == "Edit"
        }));

        assert_eq!(
            permission_additional_directories_from_settings_value(&value),
            vec!["/tmp/work".to_string(), "/tmp/other".to_string()]
        );
    }

    #[test]
    fn base_tools_create_denials_for_non_base_tools() {
        let all_tools = vec![
            "Bash".to_string(),
            "Read".to_string(),
            "Write".to_string(),
            "Agent".to_string(),
        ];

        let denials = base_tool_denials_from_cli_tools(&["Bash,Read".to_string()], &all_tools);

        assert_eq!(denials, vec!["Write".to_string(), "Agent".to_string()]);
        assert!(base_tool_denials_from_cli_tools(&["default".to_string()], &all_tools).is_empty());
    }

    #[test]
    fn model_tool_result_matches_ts_write_mapping() {
        assert_eq!(
            format_tool_result_for_model(
                "Write",
                &serde_json::json!({
                    "type": "create",
                    "filePath": "/tmp/example.txt",
                    "content": "hello",
                    "originalFile": null
                })
            ),
            "File created successfully at: /tmp/example.txt"
        );
        assert_eq!(
            format_tool_result_for_model(
                "Write",
                &serde_json::json!({
                    "type": "update",
                    "filePath": "/tmp/example.txt",
                    "content": "hello",
                    "originalFile": "old"
                })
            ),
            "The file /tmp/example.txt has been updated successfully."
        );
    }

    #[test]
    fn model_tool_result_matches_ts_edit_mapping() {
        assert_eq!(
            format_tool_result_for_model(
                "Edit",
                &serde_json::json!({
                    "filePath": "/tmp/example.txt",
                    "replaceAll": false,
                    "userModified": false
                })
            ),
            "The file /tmp/example.txt has been updated successfully."
        );
        assert_eq!(
            format_tool_result_for_model(
                "Edit",
                &serde_json::json!({
                    "filePath": "/tmp/example.txt",
                    "replaceAll": true,
                    "userModified": false
                })
            ),
            "The file /tmp/example.txt has been updated. All occurrences were successfully replaced."
        );
    }

    #[test]
    fn model_tool_result_matches_ts_todo_write_mapping() {
        assert_eq!(
            format_tool_result_for_model(
                "TodoWrite",
                &serde_json::json!({
                    "oldTodos": [],
                    "newTodos": [],
                    "verificationNudgeNeeded": false
                })
            ),
            "Todos have been modified successfully. Ensure that you continue to use the todo list to track your progress. Please proceed with the current tasks if applicable"
        );
    }

    #[test]
    fn model_tool_result_matches_ts_plan_mapping() {
        assert_eq!(
            format_tool_result_for_model(
                "EnterPlanMode",
                &serde_json::json!({
                    "message": "Entered plan mode. You should now focus on exploring the codebase and designing an implementation approach."
                })
            ),
            "Entered plan mode. You should now focus on exploring the codebase and designing an implementation approach.\n\nIn plan mode, you should:\n1. Thoroughly explore the codebase to understand existing patterns\n2. Identify similar features and architectural approaches\n3. Consider multiple approaches and their trade-offs\n4. Use AskUserQuestion if you need to clarify the approach\n5. Design a concrete implementation strategy\n6. When ready, use ExitPlanMode to present your plan for approval\n\nRemember: DO NOT write or edit any files yet. This is a read-only exploration and planning phase."
        );
        assert_eq!(
            format_tool_result_for_model(
                "ExitPlanMode",
                &serde_json::json!({
                    "plan": "",
                    "filePath": "/tmp/plan.md",
                    "hasTaskTool": true,
                    "planWasEdited": false
                })
            ),
            "User has approved exiting plan mode. You can now proceed."
        );
    }

    #[test]
    fn model_tool_result_matches_ts_web_and_config_mapping() {
        assert_eq!(
            format_tool_result_for_model(
                "WebFetch",
                &serde_json::json!({
                    "result": "Fetched page summary"
                })
            ),
            "Fetched page summary"
        );
        assert_eq!(
            format_tool_result_for_model(
                "Config",
                &serde_json::json!({
                    "action": "get",
                    "key": "theme",
                    "value": "dark"
                })
            ),
            "theme = \"dark\""
        );
        assert_eq!(
            format_tool_result_for_model(
                "Config",
                &serde_json::json!({
                    "action": "set",
                    "key": "theme",
                    "value": "dark"
                })
            ),
            "Set theme to \"dark\""
        );
    }

    #[test]
    fn model_tool_result_matches_ts_notebook_edit_mapping() {
        assert_eq!(
            format_tool_result_for_model(
                "NotebookEdit",
                &serde_json::json!({
                    "cell_id": "cell-1",
                    "edit_mode": "replace",
                    "new_source": "print(1)",
                    "error": ""
                })
            ),
            "Updated cell cell-1 with print(1)"
        );
        assert_eq!(
            format_tool_result_for_model(
                "NotebookEdit",
                &serde_json::json!({
                    "cell_id": "abc123",
                    "edit_mode": "insert",
                    "new_source": "# title",
                    "error": ""
                })
            ),
            "Inserted cell abc123 with # title"
        );
        assert_eq!(
            format_tool_result_for_model(
                "NotebookEdit",
                &serde_json::json!({
                    "cell_id": "abc123",
                    "edit_mode": "delete",
                    "new_source": "",
                    "error": ""
                })
            ),
            "Deleted cell abc123"
        );
    }

    #[test]
    fn model_tool_result_uses_ts_message_field_mapping() {
        assert_eq!(
            format_tool_result_for_model(
                "EnterWorktree",
                &serde_json::json!({
                    "worktreePath": "/tmp/wt",
                    "worktreeBranch": "worktree/demo",
                    "message": "Created worktree at /tmp/wt. The session is now working in the worktree."
                })
            ),
            "Created worktree at /tmp/wt. The session is now working in the worktree."
        );
    }

    #[test]
    fn model_tool_result_matches_ts_skill_mapping() {
        assert_eq!(
            format_tool_result_for_model(
                "Skill",
                &serde_json::json!({
                    "skill": "plugin:example",
                    "content": "large skill body",
                    "message": "Skill 'plugin:example' loaded successfully."
                })
            ),
            "Launching skill: plugin:example"
        );
    }

    #[test]
    fn model_tool_result_matches_ts_task_output_mapping() {
        assert_eq!(
            format_tool_result_for_model(
                "TaskOutput",
                &serde_json::json!({
                    "taskId": "task-1",
                    "status": "completed",
                    "output": "hello\n",
                    "pid": 123
                })
            ),
            "<retrieval_status>found</retrieval_status>\n\n<task_id>task-1</task_id>\n\n<task_type>background</task_type>\n\n<status>completed</status>\n\n<output>\nhello\n</output>"
        );
    }

    #[test]
    fn model_tool_result_matches_ts_task_family_mapping() {
        let task = serde_json::json!({
            "id": "7",
            "subject": "Wire task formatting",
            "description": "Match TS",
            "status": "pending",
            "blockedBy": [],
            "blocks": []
        });
        assert_eq!(
            format_tool_result_for_model("TaskCreate", &task),
            "Task #7 created successfully: Wire task formatting"
        );
        assert_eq!(
            format_tool_result_for_model("TaskGet", &task),
            "Task #7: Wire task formatting\nStatus: pending\nDescription: Match TS"
        );
        assert_eq!(
            format_tool_result_for_model("TaskList", &serde_json::json!({"tasks": [task]})),
            "#7 [pending] Wire task formatting"
        );
    }

    #[test]
    fn model_tool_result_matches_ts_tool_search_mapping() {
        assert_eq!(
            format_tool_result_content_for_model(
                "ToolSearch",
                &serde_json::json!({"matches": ["Read", "Grep"]})
            ),
            serde_json::json!([
                {"type": "tool_reference", "tool_name": "Read"},
                {"type": "tool_reference", "tool_name": "Grep"}
            ])
        );
        assert_eq!(
            format_tool_result_content_for_model("ToolSearch", &serde_json::json!({"matches": []})),
            serde_json::json!("No matching deferred tools found")
        );
    }

    #[test]
    fn model_tool_result_matches_ts_ask_user_mapping() {
        assert_eq!(
            format_tool_result_for_model(
                "AskUserQuestion",
                &serde_json::json!({
                    "answers": { "Which approach?": "Use Rust" },
                    "annotations": {
                        "Which approach?": {
                            "preview": "cargo test",
                            "notes": "matches the port"
                        }
                    }
                })
            ),
            "User has answered your questions: \"Which approach?\"=\"Use Rust\" selected preview:\ncargo test user notes: matches the port. You can now continue with the user's answers in mind."
        );
    }

    #[test]
    fn model_tool_result_matches_ts_agent_completed_mapping() {
        assert_eq!(
            format_tool_result_content_for_model(
                "Agent",
                &serde_json::json!({
                    "status": "completed",
                    "agentId": "agent-1",
                    "agentType": "general-purpose",
                    "content": [{ "type": "text", "text": "Done." }],
                    "totalTokens": 42,
                    "totalToolUseCount": 3,
                    "totalDurationMs": 99
                })
            ),
            serde_json::json!([
                { "type": "text", "text": "Done." },
                { "type": "text", "text": "agentId: agent-1 (use SendMessage with to: 'agent-1' to continue this agent)\n<usage>total_tokens: 42\ntool_uses: 3\nduration_ms: 99</usage>" }
            ])
        );
        assert_eq!(
            format_tool_result_content_for_model(
                "Agent",
                &serde_json::json!({
                    "status": "completed",
                    "agentId": "agent-1",
                    "agentType": "Explore",
                    "content": [{ "type": "text", "text": "Report." }],
                    "totalTokens": 42,
                    "totalToolUseCount": 3,
                    "totalDurationMs": 99
                })
            ),
            serde_json::json!([{ "type": "text", "text": "Report." }])
        );
    }

    #[test]
    fn model_tool_result_matches_ts_lsp_mapping() {
        assert_eq!(
            format_tool_result_for_model(
                "LSP",
                &serde_json::json!({
                    "operation": "hover",
                    "result": "fn main()",
                    "filePath": "src/main.rs"
                })
            ),
            "fn main()"
        );
    }
}

fn enabled_plugin_roots(
    project_root: &std::path::Path,
) -> Vec<(String, String, std::path::PathBuf)> {
    let Ok(claude_dir) = claude_core::config::paths::claude_dir() else {
        return Vec::new();
    };

    let mut roots = Vec::new();
    for plugin_id in claude_core::plugins::skill::enabled_plugins_for_project(project_root) {
        let Some((name, source)) = plugin_id.split_once('@') else {
            continue;
        };
        let cache_root = claude_dir
            .join("plugins")
            .join("cache")
            .join(source)
            .join(name);
        let Ok(entries) = std::fs::read_dir(cache_root) else {
            continue;
        };
        let mut versions: Vec<_> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect();
        versions.sort();
        if let Some(root) = versions.pop() {
            roots.push((name.to_string(), source.to_string(), root));
        }
    }
    roots
}

fn load_enabled_plugin_mcp_servers(
    project_root: &std::path::Path,
) -> (
    std::collections::HashMap<String, claude_core::config::settings::McpServerSettingsEntry>,
    Vec<String>,
) {
    let mut servers = std::collections::HashMap::new();
    let mut order = Vec::new();
    for (plugin_name, _, root) in enabled_plugin_roots(project_root) {
        let mcp_path = root.join(".mcp.json");
        let Ok(text) = std::fs::read_to_string(mcp_path) else {
            continue;
        };
        let Ok(entries) = serde_json::from_str::<
            std::collections::HashMap<
                String,
                claude_core::config::settings::McpServerSettingsEntry,
            >,
        >(&text) else {
            continue;
        };
        for (server_name, entry) in entries {
            let name = format!("plugin:{}:{}", plugin_name, server_name);
            order.push(name.clone());
            servers.insert(name, entry);
        }
    }
    (servers, order)
}

fn stream_json_plugin_entries(project_root: &std::path::Path) -> Vec<serde_json::Value> {
    enabled_plugin_roots(project_root)
        .into_iter()
        .map(|(name, source, root)| {
            serde_json::json!({
                "name": name,
                "path": root.display().to_string(),
                "source": format!("{name}@{source}"),
            })
        })
        .collect()
}

fn plugin_markdown_stems(project_root: &std::path::Path, dir_name: &str) -> Vec<String> {
    let mut names = Vec::new();
    for (plugin_name, _, root) in enabled_plugin_roots(project_root) {
        let dir = root.join(dir_name);
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        let mut stems = entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
                    return None;
                }
                path.file_stem()
                    .and_then(|stem| stem.to_str())
                    .map(|stem| stem.to_string())
            })
            .collect::<Vec<_>>();
        stems.sort();
        names.extend(
            stems
                .into_iter()
                .map(|stem| format!("{plugin_name}:{stem}")),
        );
    }
    names
}

fn stream_json_agent_names(project_root: &std::path::Path) -> Vec<String> {
    let mut agents = Vec::new();
    agents.extend(
        claude_tools::agents::definitions::builtin_agents()
            .into_iter()
            .map(|agent| agent.name),
    );
    agents.extend(plugin_markdown_stems(project_root, "agents"));
    agents.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()).then(a.cmp(b)));
    agents
}

fn stream_json_slash_commands(
    registered_skills: &[claude_tools::skill_tool::SkillEntry],
    discovered_skills: &[claude_core::plugins::types::Skill],
    mcp_prompt_commands: &[String],
) -> Vec<String> {
    let mut commands = Vec::new();

    commands.extend(stream_json_registered_skill_names(registered_skills));
    commands.extend(
        discovered_skills
            .iter()
            .filter(|skill| {
                !matches!(
                    skill.source,
                    claude_core::plugins::types::SkillSource::Plugin(_)
                )
            })
            .filter(|skill| !skill.is_plugin_command)
            .filter(|skill| skill.user_invocable)
            .map(|skill| skill.name.clone()),
    );
    commands.extend(
        discovered_skills
            .iter()
            .filter(|skill| skill.is_plugin_command)
            .filter(|skill| skill.user_invocable)
            .map(|skill| skill.name.clone()),
    );
    commands.extend(
        discovered_skills
            .iter()
            .filter(|skill| {
                matches!(
                    skill.source,
                    claude_core::plugins::types::SkillSource::Plugin(_)
                )
            })
            .filter(|skill| !skill.is_plugin_command)
            .filter(|skill| skill.user_invocable)
            .map(|skill| skill.name.clone()),
    );

    let registry = claude_core::commands::builtin::build_default_commands();
    for name in [
        "clear",
        "compact",
        "context",
        "heapdump",
        "init",
        "review",
        "security-review",
        "extra-usage",
        "usage",
        "insights",
        "team-onboarding",
    ] {
        if registry.get(name).is_some() {
            commands.push(name.to_string());
        }
    }
    commands.extend(mcp_prompt_commands.iter().cloned());

    let mut seen = std::collections::HashSet::new();
    commands
        .into_iter()
        .filter(|name| seen.insert(name.clone()))
        .collect()
}

fn stream_json_skill_names(
    registered_skills: &[claude_tools::skill_tool::SkillEntry],
    discovered_skills: &[claude_core::plugins::types::Skill],
) -> Vec<String> {
    let mut names = stream_json_registered_skill_names(registered_skills);
    let mut seen = names
        .iter()
        .cloned()
        .collect::<std::collections::HashSet<_>>();

    for name in stream_json_discovered_skill_names(discovered_skills) {
        if seen.insert(name.clone()) {
            names.push(name);
        }
    }

    names
}

fn stream_json_registered_skill_names(
    registered_skills: &[claude_tools::skill_tool::SkillEntry],
) -> Vec<String> {
    let mut names = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for skill in registered_skills {
        if skill.prompt_phase == claude_tools::skill_tool::SkillPromptPhase::StaticCommand {
            continue;
        }
        if !skill.user_invocable {
            continue;
        }
        if seen.insert(skill.name.clone()) {
            names.push(skill.name.clone());
        }
    }

    names
}

fn stream_json_discovered_skill_names(
    discovered_skills: &[claude_core::plugins::types::Skill],
) -> Vec<String> {
    let mut names = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for skill in discovered_skills {
        if skill.is_plugin_command {
            continue;
        }
        if !skill.user_invocable {
            continue;
        }
        if seen.insert(skill.name.clone()) {
            names.push(skill.name.clone());
        }
    }

    names
}

fn stream_json_api_key_source(auth: &claude_core::api::client::AuthMethod) -> &'static str {
    match auth {
        claude_core::api::client::AuthMethod::OAuthToken(_) => "none",
        claude_core::api::client::AuthMethod::ApiKey(_) => {
            if std::env::var("ANTHROPIC_API_KEY")
                .map(|value| !value.is_empty())
                .unwrap_or(false)
            {
                "env"
            } else if std::env::var("CLAUDE_CODE_API_KEY_FILE_DESCRIPTOR").is_ok() {
                "fd"
            } else {
                "user"
            }
        }
    }
}

fn stream_json_memory_paths(cwd: &std::path::Path) -> serde_json::Value {
    if !claude_core::memdir::paths::auto_memory_enabled() {
        return serde_json::json!({});
    }
    let mut auto = claude_core::memdir::paths::get_auto_mem_path(cwd)
        .display()
        .to_string();
    if !auto.ends_with(std::path::MAIN_SEPARATOR) {
        auto.push(std::path::MAIN_SEPARATOR);
    }
    serde_json::json!({ "auto": auto })
}

fn stream_json_mcp_servers_in_order(
    connections: Vec<claude_core::mcp::types::McpServerConnection>,
    order: &[String],
) -> Vec<serde_json::Value> {
    let mut by_name = connections
        .into_iter()
        .map(|conn| {
            let status = match conn.status {
                claude_core::mcp::types::McpConnectionStatus::Connected { .. } => "connected",
                claude_core::mcp::types::McpConnectionStatus::Failed { .. } => "failed",
                claude_core::mcp::types::McpConnectionStatus::Pending { .. } => "pending",
                claude_core::mcp::types::McpConnectionStatus::Disabled => "disabled",
                claude_core::mcp::types::McpConnectionStatus::NeedsAuth => "needs-auth",
            };
            (
                conn.name.clone(),
                serde_json::json!({
                    "name": conn.name,
                    "status": status,
                }),
            )
        })
        .collect::<std::collections::HashMap<_, _>>();

    let mut out = Vec::new();
    for name in order {
        if let Some(value) = by_name.remove(name) {
            out.push(value);
        }
    }
    let mut remaining = by_name.into_iter().collect::<Vec<_>>();
    remaining.sort_by(|a, b| a.0.cmp(&b.0));
    out.extend(remaining.into_iter().map(|(_, value)| value));
    out
}

#[derive(Debug, Deserialize)]
struct ClaudeAiMcpServersResponse {
    data: Vec<ClaudeAiMcpServer>,
}

#[derive(Debug, Deserialize)]
struct ClaudeAiMcpServer {
    #[serde(rename = "id")]
    _id: String,
    display_name: String,
    url: String,
}

#[derive(Debug, Default)]
struct ClaudeAiMcpDiscovery {
    configs: std::collections::HashMap<String, claude_core::mcp::types::ScopedMcpServerConfig>,
    order: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct TsToolContract {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    input_schema: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
struct ShadowMcpServer {
    name: String,
    url: Option<String>,
    needs_auth: bool,
    include_tools: bool,
}

async fn fetch_claude_ai_mcp_configs_if_eligible() -> ClaudeAiMcpDiscovery {
    use claude_core::mcp::types::{
        ConfigScope, McpHttpServerConfig, McpServerConfig, ScopedMcpServerConfig,
    };

    if is_env_defined_falsy("ENABLE_CLAUDEAI_MCP_SERVERS") {
        tracing::debug!("[claudeai-mcp] disabled by ENABLE_CLAUDEAI_MCP_SERVERS");
        return ClaudeAiMcpDiscovery::default();
    }

    let Ok(Some(tokens)) = claude_core::auth::storage::load_tokens().await else {
        tracing::debug!("[claudeai-mcp] no Claude.ai OAuth token");
        return ClaudeAiMcpDiscovery::default();
    };
    if tokens.access_token.is_empty() {
        tracing::debug!("[claudeai-mcp] empty Claude.ai OAuth token");
        return ClaudeAiMcpDiscovery::default();
    }
    if !tokens
        .scopes
        .iter()
        .any(|scope| scope == "user:mcp_servers")
    {
        tracing::debug!(
            scopes = ?tokens.scopes,
            "[claudeai-mcp] missing user:mcp_servers scope"
        );
        return ClaudeAiMcpDiscovery::default();
    }

    let url =
        claude_core::auth::login::proxy_url("https://api.anthropic.com/v1/mcp_servers?limit=1000");
    let response = claude_core::auth::login::debug_http_client()
        .get(url)
        .header("Authorization", format!("Bearer {}", tokens.access_token))
        .header("Content-Type", "application/json")
        .header("anthropic-beta", "mcp-servers-2025-12-04")
        .header("anthropic-version", "2023-06-01")
        .timeout(std::time::Duration::from_millis(5_000))
        .send()
        .await;

    let Ok(response) = response else {
        tracing::debug!("[claudeai-mcp] fetch failed");
        return ClaudeAiMcpDiscovery::default();
    };
    let Ok(payload) = response.json::<ClaudeAiMcpServersResponse>().await else {
        tracing::debug!("[claudeai-mcp] failed to decode response");
        return ClaudeAiMcpDiscovery::default();
    };

    let mut discovery = ClaudeAiMcpDiscovery::default();
    let mut used_normalized_names = std::collections::HashSet::new();
    for server in payload.data {
        let base_name = format!("claude.ai {}", server.display_name);
        let mut final_name = base_name.clone();
        let mut final_normalized = normalize_name_for_mcp(&final_name);
        let mut count = 1;
        while used_normalized_names.contains(&final_normalized) {
            count += 1;
            final_name = format!("{} ({})", base_name, count);
            final_normalized = normalize_name_for_mcp(&final_name);
        }
        used_normalized_names.insert(final_normalized);

        let mut headers = std::collections::HashMap::new();
        headers.insert(
            "Authorization".to_string(),
            format!("Bearer {}", tokens.access_token),
        );
        headers.insert(
            "User-Agent".to_string(),
            claude_core::user_agent::get_mcp_user_agent(),
        );
        headers.insert(
            "X-Mcp-Client-Session-Id".to_string(),
            claude_core::api::client::get_session_id().clone(),
        );
        let proxy_url = match claude_core::constants::oauth::get_oauth_config() {
            Ok(oauth) => format!(
                "{}{}",
                oauth.mcp_proxy_url,
                oauth.mcp_proxy_path.replace("{server_id}", &server._id)
            ),
            Err(_) => server.url,
        };

        discovery.configs.insert(
            final_name.clone(),
            ScopedMcpServerConfig {
                config: McpServerConfig::Http(McpHttpServerConfig {
                    url: proxy_url,
                    headers: Some(headers),
                }),
                scope: ConfigScope::ClaudeAi,
            },
        );
        discovery.order.push(final_name);
    }

    tracing::debug!(
        count = discovery.configs.len(),
        "[claudeai-mcp] fetched servers"
    );
    discovery
}

fn mcp_contract_shadow_tools(
    servers: impl IntoIterator<Item = ShadowMcpServer>,
) -> Vec<claude_core::mcp::manager::McpToolInfo> {
    let Ok(contracts) = serde_json::from_str::<Vec<TsToolContract>>(include_str!(
        "../../claude-tools/src/ts_tool_contracts_2_1_119.json"
    )) else {
        return Vec::new();
    };

    let server_by_normalized = servers
        .into_iter()
        .filter(|server| server.include_tools)
        .map(|server| (normalize_name_for_mcp(&server.name), server))
        .collect::<std::collections::HashMap<_, _>>();

    let mut tools = Vec::new();
    for server in server_by_normalized
        .values()
        .filter(|server| server.needs_auth)
    {
        tools.extend(mcp_auth_shadow_tools(server));
    }

    tools.extend(contracts.into_iter().filter_map(|contract| {
        let tool_name = contract.name.clone();
        let rest = tool_name.strip_prefix("mcp__")?;
        let (normalized_server, original_name) = rest.split_once("__")?;
        let server = server_by_normalized.get(normalized_server)?;
        if server.needs_auth {
            return None;
        }
        Some(claude_core::mcp::manager::McpToolInfo {
            name: contract.name,
            original_name: original_name.to_string(),
            server_name: server.name.clone(),
            description: Some(contract.description),
            input_schema: contract.input_schema,
        })
    }));
    tools
}

fn mcp_auth_shadow_tools(server: &ShadowMcpServer) -> Vec<claude_core::mcp::manager::McpToolInfo> {
    let normalized_server = normalize_name_for_mcp(&server.name);
    let server_label = server.name.as_str();
    let url = server.url.as_deref().unwrap_or("unknown URL");
    vec![
        claude_core::mcp::manager::McpToolInfo {
            name: format!("mcp__{}__authenticate", normalized_server),
            original_name: "authenticate".to_string(),
            server_name: server.name.clone(),
            description: Some(format!(
                "The `{server_label}` MCP server (claudeai-proxy at {url}) is installed but requires authentication. Call this tool to start the OAuth flow — you'll receive an authorization URL to share with the user. Once the user completes authorization in their browser, the server's real tools will become available automatically."
            )),
            input_schema: Some(serde_json::json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "additionalProperties": false,
                "properties": {},
                "type": "object",
            })),
        },
        claude_core::mcp::manager::McpToolInfo {
            name: format!("mcp__{}__complete_authentication", normalized_server),
            original_name: "complete_authentication".to_string(),
            server_name: server.name.clone(),
            description: Some(format!(
                "Complete an in-progress OAuth flow for the `{server_label}` MCP server by submitting the callback URL. Call `mcp__{normalized_server}__authenticate` first to start the flow and get the authorization URL. After the user authorizes in their browser, the browser is redirected to a `http://localhost:<port>/callback?code=...&state=...` URL — on remote sessions that page fails to load, but the URL in the address bar is still valid. Pass that full URL here as `callback_url`."
            )),
            input_schema: Some(serde_json::json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "additionalProperties": false,
                "properties": {
                    "callback_url": {
                        "description": "The full callback URL from the browser address bar after authorizing, e.g. http://localhost:<port>/callback?code=...&state=...",
                        "type": "string",
                    }
                },
                "required": ["callback_url"],
                "type": "object",
            })),
        },
    ]
}

fn mcp_config_url(config: &claude_core::mcp::types::ScopedMcpServerConfig) -> Option<String> {
    match &config.config {
        claude_core::mcp::types::McpServerConfig::Http(http) => Some(http.url.clone()),
        claude_core::mcp::types::McpServerConfig::Sse(sse)
        | claude_core::mcp::types::McpServerConfig::SseIde(sse) => Some(sse.url.clone()),
        _ => None,
    }
}

fn claude_ai_server_uses_auth_shadow(
    name: &str,
    _config: &claude_core::mcp::types::ScopedMcpServerConfig,
) -> bool {
    // TS respects the MCP needs-auth cache before attempting another remote
    // connection. Do the same here instead of inferring auth state from URL
    // substrings such as "login".
    claude_core::mcp::auth_cache::is_mcp_auth_cached(name)
}

fn dedup_claude_ai_mcp_servers(
    claude_ai_servers: std::collections::HashMap<
        String,
        claude_core::mcp::types::ScopedMcpServerConfig,
    >,
    manual_servers: &std::collections::HashMap<
        String,
        claude_core::mcp::types::ScopedMcpServerConfig,
    >,
) -> std::collections::HashMap<String, claude_core::mcp::types::ScopedMcpServerConfig> {
    let manual_sigs = manual_servers
        .iter()
        .filter_map(|(name, config)| mcp_server_signature(config).map(|sig| (sig, name.clone())))
        .collect::<std::collections::HashMap<_, _>>();

    claude_ai_servers
        .into_iter()
        .filter_map(|(name, config)| {
            if let Some(sig) = mcp_server_signature(&config) {
                if let Some(duplicate_of) = manual_sigs.get(&sig) {
                    tracing::debug!(
                        server = %name,
                        duplicate_of = %duplicate_of,
                        "suppressing duplicate claude.ai MCP connector"
                    );
                    return None;
                }
            }
            Some((name, config))
        })
        .collect()
}

fn emit_stream_json(value: serde_json::Value) {
    println!(
        "{}",
        serde_json::to_string(&value).unwrap_or_else(|_| "{}".to_string())
    );
}

fn normalize_model_display_for_stream_json(model: &str) -> String {
    model.replace("[1M]", "[1m]").replace("[2M]", "[2m]")
}

fn format_bash_tool_result_for_model(data: &serde_json::Value) -> String {
    let Some(obj) = data.as_object() else {
        return data.as_str().unwrap_or(&data.to_string()).to_string();
    };
    let stdout = obj.get("stdout").and_then(|v| v.as_str()).unwrap_or("");
    let stderr = obj.get("stderr").and_then(|v| v.as_str()).unwrap_or("");
    match (stdout.is_empty(), stderr.is_empty()) {
        (false, true) => stdout.trim_end_matches('\n').to_string(),
        (true, false) => stderr.trim_end_matches('\n').to_string(),
        (false, false) => format!("{stdout}\n{stderr}")
            .trim_end_matches('\n')
            .to_string(),
        (true, true) => String::new(),
    }
}

fn format_tool_result_for_model(tool_name: &str, data: &serde_json::Value) -> String {
    format_tool_result_string_for_model(tool_name, data)
}

fn format_tool_result_content_for_model(
    tool_name: &str,
    data: &serde_json::Value,
) -> serde_json::Value {
    if tool_name == "Agent" || tool_name == "agent" {
        return format_agent_tool_result_content_for_model(data);
    }

    if tool_name == "ToolSearch" {
        let matches = data
            .get("matches")
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|value| value.as_str().map(str::to_string))
                    .collect::<Vec<_>>()
            })
            .or_else(|| {
                data.get("tools")
                    .and_then(|value| value.as_array())
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|tool| tool.get("name").and_then(|name| name.as_str()))
                            .map(str::to_string)
                            .collect::<Vec<_>>()
                    })
            })
            .unwrap_or_default();
        if matches.is_empty() {
            let mut text = "No matching deferred tools found".to_string();
            if let Some(pending) = data
                .get("pending_mcp_servers")
                .and_then(|value| value.as_array())
                .filter(|items| !items.is_empty())
            {
                let names = pending
                    .iter()
                    .filter_map(|value| value.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                text.push_str(&format!(". Some MCP servers are still connecting: {names}. Their tools will become available shortly — try searching again."));
            }
            return serde_json::Value::String(text);
        }
        return serde_json::Value::Array(
            matches
                .into_iter()
                .map(|name| serde_json::json!({"type": "tool_reference", "tool_name": name}))
                .collect(),
        );
    }
    serde_json::Value::String(format_tool_result_string_for_model(tool_name, data))
}

fn format_agent_tool_result_content_for_model(data: &serde_json::Value) -> serde_json::Value {
    let Some(status) = data.get("status").and_then(|value| value.as_str()) else {
        return serde_json::Value::String(data.to_string());
    };

    if status == "teammate_spawned" {
        let teammate_id = data
            .get("teammate_id")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let name = data
            .get("name")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let team_name = data
            .get("team_name")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        return serde_json::json!([{
            "type": "text",
            "text": format!("Spawned successfully.\nagent_id: {teammate_id}\nname: {name}\nteam_name: {team_name}\nThe agent is now running and will receive instructions via mailbox.")
        }]);
    }

    if status == "remote_launched" {
        let task_id = data
            .get("taskId")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let session_url = data
            .get("sessionUrl")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let output_file = data
            .get("outputFile")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        return serde_json::json!([{
            "type": "text",
            "text": format!("Remote agent launched in CCR.\ntaskId: {task_id}\nsession_url: {session_url}\noutput_file: {output_file}\nThe agent is running remotely. You will be notified automatically when it completes.\nBriefly tell the user what you launched and end your response.")
        }]);
    }

    if status == "async_launched" {
        let agent_id = data
            .get("agentId")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let prefix = format!(
            "Async agent launched successfully.\nagentId: {agent_id} (internal ID - do not mention to user. Use SendMessage with to: '{agent_id}' to continue this agent.)\nThe agent is working in the background. You will be notified automatically when it completes."
        );
        let instructions = if data
            .get("canReadOutputFile")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        {
            let output_file = data
                .get("outputFile")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            format!("Do not duplicate this agent's work — avoid working with the same files or topics it is using. Work on non-overlapping tasks, or briefly tell the user what you launched and end your response.\noutput_file: {output_file}\nIf asked, you can check progress before completion by using Read or Bash tail on the output file.")
        } else {
            "Briefly tell the user what you launched and end your response. Do not generate any other text — agent results will arrive in a subsequent message.".to_string()
        };
        return serde_json::json!([{ "type": "text", "text": format!("{prefix}\n{instructions}") }]);
    }

    if status == "completed" {
        let mut content = data
            .get("content")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        if content.is_empty() {
            content.push(serde_json::json!({
                "type": "text",
                "text": "(Subagent completed but returned no output.)"
            }));
        }

        let worktree_info = match (
            data.get("worktreePath").and_then(|value| value.as_str()),
            data.get("worktreeBranch").and_then(|value| value.as_str()),
        ) {
            (Some(path), Some(branch)) => {
                format!("\nworktreePath: {path}\nworktreeBranch: {branch}")
            }
            _ => String::new(),
        };
        let one_shot = data
            .get("agentType")
            .and_then(|value| value.as_str())
            .map(|agent_type| matches!(agent_type, "Explore" | "Plan"))
            .unwrap_or(false);
        if one_shot && worktree_info.is_empty() {
            return serde_json::Value::Array(content);
        }

        let agent_id = data
            .get("agentId")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let total_tokens = data
            .get("totalTokens")
            .and_then(|value| value.as_i64())
            .unwrap_or(0);
        let total_tool_use_count = data
            .get("totalToolUseCount")
            .and_then(|value| value.as_i64())
            .unwrap_or(0);
        let total_duration_ms = data
            .get("totalDurationMs")
            .and_then(|value| value.as_i64())
            .unwrap_or(0);
        content.push(serde_json::json!({
            "type": "text",
            "text": format!("agentId: {agent_id} (use SendMessage with to: '{agent_id}' to continue this agent){worktree_info}\n<usage>total_tokens: {total_tokens}\ntool_uses: {total_tool_use_count}\nduration_ms: {total_duration_ms}</usage>")
        }));
        return serde_json::Value::Array(content);
    }

    serde_json::Value::String(data.to_string())
}

fn format_tool_result_string_for_model(tool_name: &str, data: &serde_json::Value) -> String {
    if tool_name == "Bash" {
        return format_bash_tool_result_for_model(data);
    }

    if tool_name == "AskUserQuestion" || tool_name == "AskUser" {
        let answers_text = data
            .get("answers")
            .and_then(|value| value.as_object())
            .map(|answers| {
                answers
                    .iter()
                    .map(|(question_text, answer)| {
                        let answer = answer.as_str().unwrap_or("");
                        let mut parts = vec![format!("\"{question_text}\"=\"{answer}\"")];
                        let annotation = data
                            .get("annotations")
                            .and_then(|value| value.as_object())
                            .and_then(|annotations| annotations.get(question_text));
                        if let Some(preview) = annotation
                            .and_then(|value| value.get("preview"))
                            .and_then(|value| value.as_str())
                        {
                            parts.push(format!("selected preview:\n{preview}"));
                        }
                        if let Some(notes) = annotation
                            .and_then(|value| value.get("notes"))
                            .and_then(|value| value.as_str())
                        {
                            parts.push(format!("user notes: {notes}"));
                        }
                        parts.join(" ")
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        return format!(
            "User has answered your questions: {answers_text}. You can now continue with the user's answers in mind."
        );
    }

    if tool_name == "LSP" {
        return data
            .get("result")
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| value.to_string())
            })
            .unwrap_or_else(|| data.to_string());
    }

    if tool_name == "EnterPlanMode" {
        let message = data
            .get("message")
            .and_then(|value| value.as_str())
            .unwrap_or("Entered plan mode. You should now focus on exploring the codebase and designing an implementation approach.");
        let mut content = format!(
            "{message}\n\nIn plan mode, you should:\n1. Thoroughly explore the codebase to understand existing patterns\n2. Identify similar features and architectural approaches\n3. Consider multiple approaches and their trade-offs\n4. Use AskUserQuestion if you need to clarify the approach\n5. Design a concrete implementation strategy\n6. When ready, use ExitPlanMode to present your plan for approval\n\nRemember: DO NOT write or edit any files yet. This is a read-only exploration and planning phase."
        );
        if let Some(instructions) = data.get("instructions").and_then(|value| value.as_str()) {
            content.push_str("\n\n<system-reminder>\n");
            content.push_str(instructions);
            content.push_str("\n</system-reminder>");
        }
        return content;
    }

    if tool_name == "ExitPlanMode" {
        if data
            .get("isAgent")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        {
            return "User has approved the plan. There is nothing else needed from you now. Please respond with \"ok\"".to_string();
        }
        let plan = data
            .get("plan")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        if plan.trim().is_empty() {
            return "User has approved exiting plan mode. You can now proceed.".to_string();
        }
        let file_path = data
            .get("filePath")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let team_hint = if data
            .get("hasTaskTool")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        {
            "\n\nIf this plan can be broken down into multiple independent tasks, consider using the Task tool to create a team and parallelize the work."
        } else {
            ""
        };
        let plan_label = if data
            .get("planWasEdited")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        {
            "Approved Plan (edited by user)"
        } else {
            "Approved Plan"
        };
        return format!(
            "User has approved your plan. You can now start coding. Start with updating your todo list if applicable\n\nYour plan has been saved to: {file_path}\nYou can refer back to it if needed during implementation.{team_hint}\n\n## {plan_label}:\n{plan}"
        );
    }

    if tool_name == "Write" {
        if let (Some(file_path), Some(write_type)) = (
            data.get("filePath").and_then(|value| value.as_str()),
            data.get("type").and_then(|value| value.as_str()),
        ) {
            return match write_type {
                "create" => format!("File created successfully at: {file_path}"),
                "update" => format!("The file {file_path} has been updated successfully."),
                _ => data.to_string(),
            };
        }
    }

    if tool_name == "Edit" {
        if let Some(file_path) = data.get("filePath").and_then(|value| value.as_str()) {
            let modified_note = if data
                .get("userModified")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
            {
                ".  The user modified your proposed changes before accepting them. "
            } else {
                ""
            };
            if data
                .get("replaceAll")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
            {
                return format!(
                    "The file {file_path} has been updated{modified_note}. All occurrences were successfully replaced."
                );
            }
            return format!("The file {file_path} has been updated successfully{modified_note}.");
        }
    }

    if tool_name == "TodoWrite" {
        let base = "Todos have been modified successfully. Ensure that you continue to use the todo list to track your progress. Please proceed with the current tasks if applicable";
        if data
            .get("verificationNudgeNeeded")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        {
            return format!("{base}\n\nNOTE: You just closed out 3+ tasks and none of them was a verification step. Before writing your final summary, spawn the verification agent (subagent_type=\"verification\"). You cannot self-assign PARTIAL by listing caveats in your summary — only the verifier issues a verdict.");
        }
        return base.to_string();
    }

    if tool_name == "NotebookEdit" {
        if let Some(error) = data.get("error").and_then(|value| value.as_str()) {
            if !error.is_empty() {
                return error.to_string();
            }
        }
        let cell_id = data
            .get("cell_id")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let new_source = data
            .get("new_source")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        return match data.get("edit_mode").and_then(|value| value.as_str()) {
            Some("replace") => format!("Updated cell {cell_id} with {new_source}"),
            Some("insert") => format!("Inserted cell {cell_id} with {new_source}"),
            Some("delete") => format!("Deleted cell {cell_id}"),
            _ => "Unknown edit mode".to_string(),
        };
    }

    if tool_name == "Skill" {
        if let Some(status) = data.get("status").and_then(|value| value.as_str()) {
            if status == "forked" {
                let command_name = data
                    .get("commandName")
                    .or_else(|| data.get("skill"))
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                let result = data
                    .get("result")
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                return format!(
                    "Skill \"{command_name}\" completed (forked execution).\n\nResult:\n{result}"
                );
            }
        }
        let command_name = data
            .get("commandName")
            .or_else(|| data.get("skill"))
            .and_then(|value| value.as_str())
            .unwrap_or("");
        return format!("Launching skill: {command_name}");
    }

    if tool_name == "TaskOutput" {
        let mut parts = vec![format!(
            "<retrieval_status>{}</retrieval_status>",
            data.get("retrieval_status")
                .and_then(|value| value.as_str())
                .unwrap_or("found")
        )];
        let task_value = data.get("task").unwrap_or(data);
        if let Some(task_id) = task_value
            .get("task_id")
            .or_else(|| task_value.get("taskId"))
            .and_then(|value| value.as_str())
        {
            parts.push(format!("<task_id>{task_id}</task_id>"));
            let task_type = task_value
                .get("task_type")
                .or_else(|| task_value.get("taskType"))
                .and_then(|value| value.as_str())
                .unwrap_or("background");
            parts.push(format!("<task_type>{task_type}</task_type>"));
            if let Some(status) = task_value.get("status").and_then(|value| value.as_str()) {
                parts.push(format!("<status>{status}</status>"));
            }
            if let Some(exit_code) = task_value
                .get("exitCode")
                .or_else(|| task_value.get("exit_code"))
                .and_then(|value| value.as_i64())
            {
                parts.push(format!("<exit_code>{exit_code}</exit_code>"));
            }
            if let Some(output) = task_value.get("output").and_then(|value| value.as_str()) {
                if !output.trim().is_empty() {
                    parts.push(format!("<output>\n{}\n</output>", output.trim_end()));
                }
            }
            if let Some(error) = task_value.get("error").and_then(|value| value.as_str()) {
                parts.push(format!("<error>{error}</error>"));
            }
        }
        return parts.join("\n\n");
    }

    if tool_name == "TaskCreate" {
        let task = data.get("task").unwrap_or(data);
        if let (Some(id), Some(subject)) = (
            task.get("id").and_then(|value| value.as_str()),
            task.get("subject").and_then(|value| value.as_str()),
        ) {
            return format!("Task #{id} created successfully: {subject}");
        }
    }

    if tool_name == "TaskGet" {
        let task = data.get("task").unwrap_or(data);
        if task.is_null() {
            return "Task not found".to_string();
        }
        if let Some(id) = task.get("id").and_then(|value| value.as_str()) {
            let subject = task
                .get("subject")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let status = task
                .get("status")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let description = task
                .get("description")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let mut lines = vec![
                format!("Task #{id}: {subject}"),
                format!("Status: {status}"),
                format!("Description: {description}"),
            ];
            if let Some(blocked_by) = task.get("blockedBy").and_then(|value| value.as_array()) {
                if !blocked_by.is_empty() {
                    lines.push(format!(
                        "Blocked by: {}",
                        blocked_by
                            .iter()
                            .filter_map(|value| value.as_str().map(|id| format!("#{id}")))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            }
            if let Some(blocks) = task.get("blocks").and_then(|value| value.as_array()) {
                if !blocks.is_empty() {
                    lines.push(format!(
                        "Blocks: {}",
                        blocks
                            .iter()
                            .filter_map(|value| value.as_str().map(|id| format!("#{id}")))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            }
            return lines.join("\n");
        }
    }

    if tool_name == "TaskList" {
        if let Some(tasks) = data.get("tasks").and_then(|value| value.as_array()) {
            if tasks.is_empty() {
                return "No tasks found".to_string();
            }
            return tasks
                .iter()
                .filter_map(|task| {
                    let id = task.get("id").and_then(|value| value.as_str())?;
                    let status = task.get("status").and_then(|value| value.as_str())?;
                    let subject = task.get("subject").and_then(|value| value.as_str())?;
                    let owner = task
                        .get("owner")
                        .and_then(|value| value.as_str())
                        .map(|owner| format!(" ({owner})"))
                        .unwrap_or_default();
                    let blocked = task
                        .get("blockedBy")
                        .and_then(|value| value.as_array())
                        .filter(|items| !items.is_empty())
                        .map(|items| {
                            format!(
                                " [blocked by {}]",
                                items
                                    .iter()
                                    .filter_map(|value| value.as_str().map(|id| format!("#{id}")))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            )
                        })
                        .unwrap_or_default();
                    Some(format!("#{id} [{status}] {subject}{owner}{blocked}"))
                })
                .collect::<Vec<_>>()
                .join("\n");
        }
    }

    if tool_name == "WebFetch" {
        if let Some(result) = data.get("result").and_then(|value| value.as_str()) {
            return result.to_string();
        }
    }

    if tool_name == "WebSearch" {
        if let Some(query) = data.get("query").and_then(|value| value.as_str()) {
            let mut formatted = format!("Web search results for query: \"{query}\"\n\n");
            if let Some(results) = data.get("results").and_then(|value| value.as_array()) {
                for result in results {
                    if result.is_null() {
                        continue;
                    }
                    if let Some(text) = result.as_str() {
                        formatted.push_str(text);
                        formatted.push_str("\n\n");
                    } else if result
                        .get("content")
                        .and_then(|value| value.as_array())
                        .map(|content| !content.is_empty())
                        .unwrap_or(false)
                    {
                        formatted.push_str("Links: ");
                        formatted.push_str(
                            &serde_json::to_string(result.get("content").unwrap())
                                .unwrap_or_else(|_| "[]".to_string()),
                        );
                        formatted.push_str("\n\n");
                    } else {
                        formatted.push_str("No links found.\n\n");
                    }
                }
            }
            formatted.push_str(
                "\nREMINDER: You MUST include the sources above in your response to the user using markdown hyperlinks.",
            );
            return formatted.trim().to_string();
        }
    }

    if tool_name == "Config" {
        if let Some(error) = data.get("error").and_then(|value| value.as_str()) {
            return format!("Error: {error}");
        }
        let action = data
            .get("operation")
            .or_else(|| data.get("action"))
            .and_then(|value| value.as_str());
        let setting = data
            .get("setting")
            .or_else(|| data.get("key"))
            .and_then(|value| value.as_str());
        if let (Some(action), Some(setting)) = (action, setting) {
            if action == "get" {
                let value = data.get("value").unwrap_or(&serde_json::Value::Null);
                return format!("{setting} = {}", json_stringify_for_ts(value));
            }
            if action == "set" {
                let value = data
                    .get("newValue")
                    .or_else(|| data.get("value"))
                    .unwrap_or(&serde_json::Value::Null);
                return format!("Set {setting} to {}", json_stringify_for_ts(value));
            }
        }
    }

    if let Some(text) = data.as_str() {
        return text.to_string();
    }

    if data.get("type").and_then(|value| value.as_str()) == Some("text") {
        if let Some(content) = data
            .get("file")
            .and_then(|file| file.get("content"))
            .and_then(|content| content.as_str())
        {
            return content.to_string();
        }
        if let Some(content) = data.get("content").and_then(|content| content.as_str()) {
            return content.to_string();
        }
    }

    if let Some(mode) = data.get("mode").and_then(|value| value.as_str()) {
        let content = data
            .get("content")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let limit_info = format_search_limit_info(data);
        match mode {
            "content" => {
                let result = if content.is_empty() {
                    "No matches found".to_string()
                } else {
                    content.to_string()
                };
                return if let Some(limit_info) = limit_info {
                    format!("{result}\n\n[Showing results with pagination = {limit_info}]")
                } else {
                    result
                };
            }
            "count" => {
                let raw_content = if content.is_empty() {
                    "No matches found".to_string()
                } else {
                    content.to_string()
                };
                let matches = data
                    .get("numMatches")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0);
                let files = data
                    .get("numFiles")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0);
                let occurrence = if matches == 1 {
                    "occurrence"
                } else {
                    "occurrences"
                };
                let file = if files == 1 { "file" } else { "files" };
                let mut summary =
                    format!("\n\nFound {matches} total {occurrence} across {files} {file}.");
                if let Some(limit_info) = limit_info {
                    summary.push_str(&format!(" with pagination = {limit_info}"));
                }
                return raw_content + &summary;
            }
            _ => {
                let filenames = data
                    .get("filenames")
                    .and_then(|value| value.as_array())
                    .map(|values| {
                        values
                            .iter()
                            .filter_map(|value| value.as_str().map(str::to_string))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let files = data
                    .get("numFiles")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(filenames.len() as u64);
                if files == 0 {
                    return "No files found".to_string();
                }
                let file = if files == 1 { "file" } else { "files" };
                let limit = limit_info
                    .map(|info| format!(" {info}"))
                    .unwrap_or_default();
                return format!("Found {files} {file}{limit}\n{}", filenames.join("\n"));
            }
        }
    }

    if let Some(filenames) = data.get("filenames").and_then(|value| value.as_array()) {
        let mut lines = filenames
            .iter()
            .filter_map(|value| value.as_str().map(str::to_string))
            .collect::<Vec<_>>();
        if data
            .get("truncated")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        {
            lines.push(
                "(Results are truncated. Consider using a more specific path or pattern.)"
                    .to_string(),
            );
        }
        return if lines.is_empty() {
            "No files found".to_string()
        } else {
            lines.join("\n")
        };
    }

    if let Some(message) = data.get("message").and_then(|value| value.as_str()) {
        return message.to_string();
    }

    if let Some(error) = data.get("error").and_then(|value| value.as_str()) {
        return error.to_string();
    }

    data.to_string()
}

fn format_search_limit_info(data: &serde_json::Value) -> Option<String> {
    let limit = data.get("appliedLimit").and_then(|value| value.as_u64());
    let offset = data
        .get("appliedOffset")
        .and_then(|value| value.as_u64())
        .unwrap_or(0);
    limit.map(|limit| {
        if offset > 0 {
            format!("limit: {limit}, offset: {offset}")
        } else {
            format!("limit: {limit}")
        }
    })
}

fn json_stringify_for_ts(value: &serde_json::Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
}

fn permission_mode_hook_name(
    mode: &claude_core::permissions::types::PermissionMode,
) -> &'static str {
    use claude_core::permissions::types::PermissionMode;
    match mode {
        PermissionMode::Default => "default",
        PermissionMode::AcceptEdits => "acceptEdits",
        PermissionMode::Auto => "auto",
        PermissionMode::Plan => "plan",
        PermissionMode::BypassPermissions => "bypassPermissions",
        PermissionMode::DontAsk => "dontAsk",
        PermissionMode::Bubble => "bubble",
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PermissionPromptToolOutput {
    behavior: String,
    #[serde(default)]
    updated_input: Option<serde_json::Value>,
    #[serde(default)]
    updated_permissions: Option<Vec<claude_core::permissions::types::PermissionUpdate>>,
    #[serde(default)]
    message: Option<String>,
}

fn prompt_tool_output_text(data: &serde_json::Value) -> anyhow::Result<&str> {
    data.as_str().ok_or_else(|| {
        anyhow::anyhow!(
            "Permission prompt tool returned an invalid result. Expected a single text block param with type=\"text\" and a string text value."
        )
    })
}

async fn run_permission_prompt_tool(
    permission_prompt_tool: &Arc<dyn claude_tools::ToolExecutor>,
    target_tool_name: &str,
    target_tool_use_id: &str,
    target_input: &serde_json::Value,
    cwd: &std::path::Path,
    read_file_state: Arc<std::sync::Mutex<claude_tools::registry::ReadFileState>>,
    permission_mode: claude_core::permissions::types::PermissionMode,
    model: &str,
    cancel: tokio_util::sync::CancellationToken,
) -> anyhow::Result<PermissionPromptToolOutput> {
    let ctx = claude_tools::ToolUseContext::new(
        cwd.to_path_buf(),
        read_file_state,
        permission_mode,
        std::sync::Arc::new(
            claude_core::tool_use_context_options::ToolUseContextOptions::minimal(model),
        ),
        std::sync::Arc::new(claude_core::tool_host::NullToolHost),
    );
    let prompt_input = serde_json::json!({
        "tool_name": target_tool_name,
        "input": target_input,
        "tool_use_id": target_tool_use_id,
    });
    let result = permission_prompt_tool
        .call(&prompt_input, &ctx, cancel, None)
        .await?;
    let text = prompt_tool_output_text(&result.data)?;
    let parsed: PermissionPromptToolOutput = serde_json::from_str(text)?;
    Ok(parsed)
}

fn merge_hook_updated_input(
    input: &serde_json::Value,
    updated: &Option<std::collections::HashMap<String, serde_json::Value>>,
) -> serde_json::Value {
    let Some(updated) = updated else {
        return input.clone();
    };
    let mut merged = input.clone();
    if let Some(obj) = merged.as_object_mut() {
        for (key, value) in updated {
            obj.insert(key.clone(), value.clone());
        }
    }
    merged
}

fn permission_decision_to_rule_check(
    decision: &claude_core::permissions::types::PermissionDecision,
) -> claude_core::hooks::RuleCheckResult {
    use claude_core::hooks::RuleCheckResult;
    use claude_core::permissions::types::PermissionDecision;
    match decision {
        PermissionDecision::Allow(_) => RuleCheckResult::NoMatch,
        PermissionDecision::Ask(_) => RuleCheckResult::Ask,
        PermissionDecision::Deny(deny) => RuleCheckResult::Deny(Some(deny.message.clone())),
    }
}

fn hook_blocking_errors_text(errors: &[claude_core::hooks::HookBlockingError]) -> Option<String> {
    if errors.is_empty() {
        None
    } else {
        Some(
            errors
                .iter()
                .map(|err| err.blocking_error.clone())
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }
}

fn stream_event_to_stream_json(
    ev: &claude_core::types::events::StreamEvent,
    session_id: &str,
) -> Option<serde_json::Value> {
    use claude_core::types::events::StreamEvent;
    use serde_json::json;

    let value = match ev {
        // TS stream-json exposes model messages, user tool-result turns, system
        // records, and the final result. It does not print Rust's internal
        // request/delta/progress bookkeeping events as first-class records.
        StreamEvent::RequestStart { .. }
        | StreamEvent::ToolStart { .. }
        | StreamEvent::ToolProgress { .. }
        | StreamEvent::ToolResult { .. }
        | StreamEvent::ThinkingDelta { .. }
        | StreamEvent::TextDelta { .. }
        | StreamEvent::UsageUpdate(_)
        | StreamEvent::Done { .. } => return None,
        StreamEvent::AssistantMessage(message) => {
            json!({
                "type": "assistant",
                "message": serde_json::to_value(&message.message).unwrap_or(serde_json::Value::Null),
                "parent_tool_use_id": serde_json::Value::Null,
                "session_id": session_id,
                "uuid": message.uuid.to_string(),
            })
        }
        StreamEvent::Compacted { summary } => json!({
            "type": "system",
            "subtype": "compact_boundary",
            "session_id": session_id,
            "uuid": uuid::Uuid::new_v4().to_string(),
            "timestamp": chrono::Utc::now(),
            "message": {"summary": summary},
        }),
        StreamEvent::Error(error) => json!({"type": "error", "error": format!("{:?}", error)}),
    };
    Some(value)
}

fn stream_json_user_tool_result_event(
    tool_results: Vec<serde_json::Value>,
    session_id: &str,
) -> serde_json::Value {
    serde_json::json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": tool_results,
        },
        "session_id": session_id,
        "uuid": uuid::Uuid::new_v4().to_string(),
    })
}

#[derive(Default)]
struct StreamJsonPrintState {
    text: String,
    latest_usage: Option<claude_core::types::usage::Usage>,
}

fn merge_stream_usage(
    current: &mut Option<claude_core::types::usage::Usage>,
    update: claude_core::types::usage::Usage,
) {
    match current {
        Some(existing) => {
            if update.input_tokens > 0 {
                existing.input_tokens = update.input_tokens;
            }
            existing.output_tokens = existing.output_tokens.max(update.output_tokens);
            if update.cache_creation_input_tokens.is_some() {
                existing.cache_creation_input_tokens = update.cache_creation_input_tokens;
            }
            if update.cache_read_input_tokens.is_some() {
                existing.cache_read_input_tokens = update.cache_read_input_tokens;
            }
        }
        None => *current = Some(update),
    }
}

struct StreamJsonResultMeta<'a> {
    duration_ms: u128,
    num_turns: u32,
    stop_reason: &'a str,
    usage: Option<&'a claude_core::types::usage::Usage>,
    model_display: &'a str,
    max_tokens: u64,
    context_window: u64,
    total_cost_usd: f64,
}

fn stream_json_result_event_with_meta(
    final_text: &str,
    session_id: &str,
    meta: StreamJsonResultMeta<'_>,
) -> serde_json::Value {
    let usage = meta.usage.map(|usage| {
        serde_json::json!({
            "input_tokens": usage.input_tokens,
            "output_tokens": usage.output_tokens,
            "cache_creation_input_tokens": usage.cache_creation_input_tokens.unwrap_or(0),
            "cache_read_input_tokens": usage.cache_read_input_tokens.unwrap_or(0),
            "server_tool_use": {
                "web_search_requests": 0,
                "web_fetch_requests": 0,
            },
            "iterations": [{
                "input_tokens": usage.input_tokens,
                "output_tokens": usage.output_tokens,
                "cache_creation_input_tokens": usage.cache_creation_input_tokens.unwrap_or(0),
                "cache_read_input_tokens": usage.cache_read_input_tokens.unwrap_or(0),
                "type": "message",
            }],
        })
    });
    let model_usage = meta.usage.map(|usage| {
        let mut map = serde_json::Map::new();
        map.insert(
            normalize_model_display_for_stream_json(meta.model_display),
            serde_json::json!({
                "inputTokens": usage.input_tokens,
                "outputTokens": usage.output_tokens,
                "cacheReadInputTokens": usage.cache_read_input_tokens.unwrap_or(0),
                "cacheCreationInputTokens": usage.cache_creation_input_tokens.unwrap_or(0),
                "webSearchRequests": 0,
                "costUSD": meta.total_cost_usd,
                "contextWindow": meta.context_window,
                "maxOutputTokens": meta.max_tokens,
            }),
        );
        serde_json::Value::Object(map)
    });

    serde_json::json!({
        "type": "result",
        "subtype": "success",
        "is_error": false,
        "api_error_status": serde_json::Value::Null,
        "duration_ms": meta.duration_ms,
        "duration_api_ms": meta.duration_ms,
        "num_turns": meta.num_turns,
        "result": final_text,
        "stop_reason": meta.stop_reason,
        "session_id": session_id,
        "total_cost_usd": meta.total_cost_usd,
        "usage": usage,
        "modelUsage": model_usage,
        "permission_denials": [],
        "terminal_reason": "completed",
        "fast_mode_state": "off",
        "uuid": uuid::Uuid::new_v4().to_string(),
    })
}

struct StreamJsonInitMeta<'a> {
    cwd: &'a std::path::Path,
    session_id: &'a str,
    tool_names: Vec<String>,
    mcp_servers: Vec<serde_json::Value>,
    model_display: &'a str,
    permission_mode: &'a claude_core::permissions::types::PermissionMode,
    registered_skills: &'a [claude_tools::skill_tool::SkillEntry],
    discovered_skills: &'a [claude_core::plugins::types::Skill],
    stream_skill_names: &'a [String],
    mcp_prompt_commands: &'a [String],
    output_style: Option<&'a str>,
    auth: &'a claude_core::api::client::AuthMethod,
}

fn stream_json_init_event(meta: StreamJsonInitMeta<'_>) -> serde_json::Value {
    serde_json::json!({
        "type": "system",
        "subtype": "init",
        "cwd": meta.cwd.display().to_string(),
        "session_id": meta.session_id,
        "tools": meta.tool_names,
        "mcp_servers": meta.mcp_servers,
        "model": normalize_model_display_for_stream_json(meta.model_display),
        "permissionMode": serde_json::to_value(meta.permission_mode).unwrap_or(serde_json::json!("default")),
        "slash_commands": stream_json_slash_commands(meta.registered_skills, meta.discovered_skills, meta.mcp_prompt_commands),
        "apiKeySource": stream_json_api_key_source(meta.auth),
        "claude_code_version": env!("CARGO_PKG_VERSION"),
        "output_style": meta.output_style.unwrap_or("default"),
        "agents": stream_json_agent_names(meta.cwd),
        "skills": meta.stream_skill_names,
        "plugins": stream_json_plugin_entries(meta.cwd),
        "analytics_disabled": claude_core::privacy_level::is_telemetry_disabled(),
        "memory_paths": stream_json_memory_paths(meta.cwd),
        "fast_mode_state": "off",
        "uuid": uuid::Uuid::new_v4().to_string(),
    })
}

fn emit_stream_json_hook_events(result: &claude_core::hooks::types::HookResult, session_id: &str) {
    let hook_id = result
        .hook_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let hook_name = result.hook_name.as_deref().unwrap_or("Hook");
    let hook_event = result.hook_event.as_deref().unwrap_or("Hook");

    emit_stream_json(serde_json::json!({
        "type": "system",
        "subtype": "hook_started",
        "hook_id": hook_id,
        "hook_name": hook_name,
        "hook_event": hook_event,
        "uuid": uuid::Uuid::new_v4().to_string(),
        "session_id": session_id,
    }));

    let output = if !result.stdout.is_empty() {
        result.stdout.as_str()
    } else {
        result.stderr.as_str()
    };
    emit_stream_json(serde_json::json!({
        "type": "system",
        "subtype": "hook_response",
        "hook_id": hook_id,
        "hook_name": hook_name,
        "hook_event": hook_event,
        "output": output,
        "stdout": result.stdout,
        "stderr": result.stderr,
        "exit_code": result.exit_code,
        "outcome": result.outcome.to_string(),
        "uuid": uuid::Uuid::new_v4().to_string(),
        "session_id": session_id,
    }));
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut prompt_arg = cli.prompt.clone();
    if cli.print && prompt_arg.is_none() {
        use std::io::Read;
        let mut stdin = String::new();
        std::io::stdin().read_to_string(&mut stdin)?;
        let stdin = stdin.trim_end().to_string();
        if !stdin.is_empty() {
            prompt_arg = Some(stdin);
        }
    }

    // Set working directory if specified
    if let Some(dir) = &cli.working_dir {
        std::env::set_current_dir(dir)?;
    }
    let is_interactive_session = prompt_arg.is_none();

    // Initialize tracing. Stream-json stdout must remain valid JSONL; TS does
    // not interleave normal log lines with stream-json records.
    let trace_filter = if cli.output_format == OutputFormat::StreamJson && prompt_arg.is_some() {
        "off"
    } else if cli.verbose {
        "debug"
    } else {
        "error"
    };
    tracing_subscriber::fmt()
        .with_env_filter(trace_filter)
        .init();

    // Handle subcommands
    match &cli.command {
        Some(SubCommand::Login {
            email,
            sso,
            console,
            claudeai,
        }) => {
            let opts = claude_core::auth::login::LoginOptions {
                email: email.clone(),
                sso: *sso,
                use_console: *console,
                use_claude_ai: *claudeai || !*console, // Default to Claude.ai if neither flag
            };
            claude_core::auth::login::login_with_options(opts).await?;
            return Ok(());
        }
        Some(SubCommand::Logout) => {
            // Delete tokens from secure storage (keychain + file)
            claude_core::auth::storage::delete_tokens().await.ok();
            claude_core::auth::storage::delete_managed_api_key()
                .await
                .ok();
            // Clear account info from global config
            claude_core::config::global::save_global_config(|mut config| {
                config.oauth_account = None;
                config.primary_api_key = None;
                config
            })
            .ok();
            println!("Successfully logged out from your Anthropic account.");
            return Ok(());
        }
        Some(SubCommand::Config) => {
            let cwd = std::env::current_dir()?;
            let root = claude_core::config::paths::detect_project_root(&cwd);
            println!("Project root: {}", root.display());
            println!(
                "Config dir: {}",
                claude_core::config::paths::claude_dir()?.display()
            );
            return Ok(());
        }
        Some(SubCommand::Server { port }) => {
            // Start the IDE bridge server
            let config = claude_core::bridge::types::BridgeConfig {
                port: *port,
                host: "127.0.0.1".to_string(),
                ide: claude_core::bridge::types::IdeType::Other("cli".to_string()),
            };
            let server = claude_core::bridge::server::BridgeServer::new(config);
            tracing::info!("Starting IDE bridge server...");
            server.start().await?;
            return Ok(());
        }
        Some(SubCommand::RemoteControl { name }) => {
            let name_suffix = name
                .as_deref()
                .filter(|value| !value.is_empty())
                .map(|value| format!(" for `{value}`"))
                .unwrap_or_default();
            eprintln!(
                "Remote Control{name_suffix} is not fully ported in claude-rs yet.\n\n\
                 The original TS path starts a Claude.ai bridge runtime here: \
                 entitlement/policy checks, environment registration, session creation, \
                 session-ingress WebSocket forwarding, and inbound prompt queueing.\n\n\
                 Current Rust support is limited to the local IDE bridge server via \
                 `claude-rs server --port <N>` and the in-TUI `/remote-control` status command."
            );
            std::process::exit(1);
        }
        None => {}
    }

    // Resolve authentication (reads keychain, refreshes OAuth tokens)
    let auth = match claude_core::auth::resolve::resolve_auth().await {
        Ok(auth) => auth,
        Err(e) => {
            eprintln!();
            eprintln!("  \x1b[1mWelcome to Claude Code!\x1b[0m");
            eprintln!();
            eprintln!("  {}", e);
            eprintln!();
            eprintln!("  To get started:");
            eprintln!("  1. Run \x1b[1mclaude-rs login\x1b[0m");
            eprintln!("  2. Or set: \x1b[1mexport ANTHROPIC_API_KEY=sk-ant-...\x1b[0m");
            eprintln!();
            std::process::exit(1);
        }
    };

    // Load settings
    let cwd = std::env::current_dir()?;
    let project_root = claude_core::config::paths::detect_project_root(&cwd);

    // Prime the undercover repo-class cache early so `is_undercover()`
    // can resolve against a concrete classification before the first
    // tool description or commit prompt is rendered. Matches TS
    // setup.ts which calls isInternalModelRepo() at startup. Ant-only:
    // for other user types the reader short-circuits so the subprocess
    // cost is wasted — gate here. Fire-and-forget: `prime_repo_class
    // _from_remote` is idempotent and the conservative-safe default
    // (undercover ON when unprimed) covers the race if anything reads
    // the cache before this task completes.
    if claude_core::user_type::is_ant() {
        let prime_root = project_root.clone();
        tokio::task::spawn_blocking(move || {
            claude_core::undercover::prime_repo_class_from_remote(&prime_root);
        });
    }

    // Load settings from ~/.claude/settings.json
    let settings = match claude_core::config::paths::user_settings_path() {
        Ok(path) => claude_core::config::settings::Settings::load_from_file(&path),
        Err(_) => claude_core::config::settings::Settings::default(),
    };
    let raw_settings =
        claude_core::permissions::load_raw_settings_value_with_plugin_hooks(&project_root);
    let permission_settings =
        claude_core::permissions::load_permission_settings_value(&project_root);

    // Build tool registry
    let mut tools =
        claude_tools::build_default_registry_with_options(claude_tools::RegistryOptions {
            is_non_interactive_session: prompt_arg.is_some(),
        });
    let base_permission_tool_names = tools
        .all()
        .iter()
        .map(|tool| tool.name().to_string())
        .collect::<Vec<_>>();
    filter_registry_by_cli_tools(&mut tools, &cli.tools);

    // Register bundled skills (simplify, stuck, remember, …).
    // Each skill's registrar applies its own TS-parity gate
    // (user-type, feature flag) so calling this unconditionally
    // at startup is safe. Matches TS setup.ts which calls each
    // registerXxxSkill() at startup and lets the skill choose
    // whether to register.
    claude_tools::bundled_skills::register_bundled_skills();

    // --- MCP server wiring ---
    // Connect to MCP servers configured in settings.mcpServers
    let mcp_manager = Arc::new(RwLock::new(claude_core::mcp::manager::McpManager::new()));
    let mut mcp_server_settings = settings.mcp_servers.clone();
    let (plugin_mcp_servers, plugin_mcp_order) = load_enabled_plugin_mcp_servers(&project_root);
    mcp_server_settings.extend(plugin_mcp_servers);
    let mut configs = std::collections::HashMap::new();
    let mut mcp_server_order = plugin_mcp_order;
    let mut plugin_mcp_server_names = Vec::new();
    for (name, entry) in &mcp_server_settings {
        if !mcp_server_order.contains(name) {
            mcp_server_order.push(name.clone());
        }
        let env = if entry.env.is_empty() {
            None
        } else {
            Some(entry.env.clone())
        };
        let scoped = claude_core::mcp::types::ScopedMcpServerConfig {
            config: claude_core::mcp::types::McpServerConfig::Stdio(
                claude_core::mcp::types::McpStdioServerConfig {
                    command: entry.command.clone(),
                    args: entry.args.clone(),
                    env,
                },
            ),
            scope: claude_core::mcp::types::ConfigScope::User,
        };
        if name.starts_with("plugin:") {
            plugin_mcp_server_names.push(name.clone());
        }
        configs.insert(name.clone(), scoped);
    }
    let ClaudeAiMcpDiscovery {
        configs: discovered_claude_ai_configs,
        order: discovered_claude_ai_order,
    } = fetch_claude_ai_mcp_configs_if_eligible().await;
    let claude_ai_configs = dedup_claude_ai_mcp_servers(discovered_claude_ai_configs, &configs);
    for name in discovered_claude_ai_order {
        if claude_ai_configs.contains_key(&name) && !mcp_server_order.contains(&name) {
            mcp_server_order.push(name);
        }
    }
    let claude_ai_shadow_servers = claude_ai_configs
        .iter()
        .map(|(name, config)| ShadowMcpServer {
            name: name.clone(),
            url: mcp_config_url(config),
            needs_auth: claude_ai_server_uses_auth_shadow(name, config),
            include_tools: true,
        })
        .collect::<Vec<_>>();
    configs.extend(claude_ai_configs);
    let mut claude_ai_shadow_tools = Vec::new();
    claude_ai_shadow_tools.extend(mcp_contract_shadow_tools(
        plugin_mcp_server_names
            .into_iter()
            .map(|name| ShadowMcpServer {
                name,
                url: None,
                needs_auth: false,
                include_tools: true,
            }),
    ));
    if !configs.is_empty() {
        tracing::info!(
            count = configs.len(),
            "Connecting to configured MCP servers"
        );

        if is_interactive_session {
            let manager = mcp_manager.clone();
            let claude_ai_shadow_servers = claude_ai_shadow_servers.clone();
            tokio::spawn(async move {
                let mgr = manager.read().await;
                let connections = mgr.connect_all_respecting_auth_cache(configs).await;
                let connected = connections
                    .iter()
                    .filter(|conn| {
                        matches!(
                            conn.status,
                            claude_core::mcp::types::McpConnectionStatus::Connected { .. }
                        )
                    })
                    .map(|conn| conn.name.clone())
                    .collect::<std::collections::HashSet<_>>();
                let needs_auth = connections
                    .iter()
                    .filter(|conn| {
                        matches!(
                            conn.status,
                            claude_core::mcp::types::McpConnectionStatus::NeedsAuth
                        )
                    })
                    .map(|conn| conn.name.clone())
                    .collect::<std::collections::HashSet<_>>();
                let auth_shadow_tools = mcp_contract_shadow_tools(
                    claude_ai_shadow_servers.into_iter().map(|mut server| {
                        let is_connected = connected.contains(&server.name);
                        server.needs_auth = needs_auth.contains(&server.name)
                            || (server.needs_auth && !is_connected);
                        server.include_tools = is_connected || server.needs_auth;
                        server
                    }),
                );
                if !auth_shadow_tools.is_empty() {
                    mgr.add_tool_definitions(auth_shadow_tools).await;
                }
                drop(mgr);
                for conn in &connections {
                    match &conn.status {
                        claude_core::mcp::types::McpConnectionStatus::Connected { .. } => {
                            tracing::info!(server = %conn.name, "MCP server connected");
                        }
                        claude_core::mcp::types::McpConnectionStatus::Failed { error } => {
                            tracing::warn!(
                                server = %conn.name,
                                error = ?error,
                                "MCP server failed to connect"
                            );
                        }
                        _ => {}
                    }
                }
            });
        } else {
            let mgr = mcp_manager.read().await;
            let connections = mgr.connect_all_respecting_auth_cache(configs).await;
            let connected = connections
                .iter()
                .filter(|conn| {
                    matches!(
                        conn.status,
                        claude_core::mcp::types::McpConnectionStatus::Connected { .. }
                    )
                })
                .map(|conn| conn.name.clone())
                .collect::<std::collections::HashSet<_>>();
            let needs_auth = connections
                .iter()
                .filter(|conn| {
                    matches!(
                        conn.status,
                        claude_core::mcp::types::McpConnectionStatus::NeedsAuth
                    )
                })
                .map(|conn| conn.name.clone())
                .collect::<std::collections::HashSet<_>>();
            claude_ai_shadow_tools.extend(mcp_contract_shadow_tools(
                claude_ai_shadow_servers.into_iter().map(|mut server| {
                    let is_connected = connected.contains(&server.name);
                    server.needs_auth =
                        needs_auth.contains(&server.name) || (server.needs_auth && !is_connected);
                    server.include_tools = is_connected || server.needs_auth;
                    server
                }),
            ));
            drop(mgr);

            // Report connection results
            for conn in &connections {
                match &conn.status {
                    claude_core::mcp::types::McpConnectionStatus::Connected { .. } => {
                        tracing::info!(server = %conn.name, "MCP server connected");
                    }
                    claude_core::mcp::types::McpConnectionStatus::Failed { error } => {
                        tracing::warn!(
                            server = %conn.name,
                            error = ?error,
                            "MCP server failed to connect"
                        );
                    }
                    _ => {}
                }
            }
        }

        if !claude_ai_shadow_tools.is_empty() {
            let mgr = mcp_manager.read().await;
            mgr.add_tool_definitions(claude_ai_shadow_tools).await;
        }

        // Register immediately available MCP/shadow tools into the registry.
        claude_tools::register_mcp_tools(&mut tools, mcp_manager.clone()).await;
    }
    filter_registry_by_cli_tools(&mut tools, &cli.tools);
    claude_tools::filter_registry_by_deny_rules(&mut tools, &settings.permissions.deny);
    let permission_prompt_tool = if let Some(name) = &cli.permission_prompt_tool {
        if prompt_arg.is_none() {
            eprintln!("Error: --permission-prompt-tool can only be used with --print");
            std::process::exit(1);
        }
        let Some(tool) = tools.remove(name) else {
            let available_mcp_tools = tools
                .all()
                .iter()
                .filter(|tool| tool.name().starts_with("mcp__"))
                .map(|tool| tool.name().to_string())
                .collect::<Vec<_>>();
            eprintln!(
                "Error: MCP tool {} (passed via --permission-prompt-tool) not found. Available MCP tools: {}",
                name,
                if available_mcp_tools.is_empty() {
                    "none".to_string()
                } else {
                    available_mcp_tools.join(", ")
                }
            );
            std::process::exit(1);
        };
        if !tool.name().starts_with("mcp__") {
            eprintln!(
                "Error: tool {} (passed via --permission-prompt-tool) must be an MCP tool",
                name
            );
            std::process::exit(1);
        }
        Some(tool)
    } else {
        None
    };
    claude_tools::register_tool_search_snapshot(&mut tools);

    // --- Skill discovery ---
    let discovered_skills = claude_core::plugins::skill::discover_skills(&project_root);
    let registered_skills = claude_tools::skill_tool::list_skills();
    let stream_skill_names = stream_json_skill_names(&registered_skills, &discovered_skills);
    let mut skills = Vec::new();
    {
        let mut seen = std::collections::HashSet::new();
        for skill in &registered_skills {
            if skill.prompt_phase == claude_tools::skill_tool::SkillPromptPhase::StaticCommand {
                continue;
            }
            if skill.disable_model_invocation {
                continue;
            }
            if !seen.insert(skill.name.clone()) {
                continue;
            }
            skills.push(claude_core::plugins::types::Skill {
                name: skill.name.clone(),
                description: skill.description.clone(),
                content: skill.content.clone(),
                source: claude_core::plugins::types::SkillSource::Builtin,
                argument_hint: None,
                when_to_use: None,
                allowed_tools: Vec::new(),
                user_invocable: skill.user_invocable,
                disable_model_invocation: skill.disable_model_invocation,
                is_plugin_command: false,
            });
        }
        for skill in discovered_skills.iter().cloned() {
            if skill.disable_model_invocation {
                continue;
            }
            if seen.insert(skill.name.clone()) {
                skills.push(skill);
            }
        }
        for skill in &registered_skills {
            if skill.prompt_phase != claude_tools::skill_tool::SkillPromptPhase::StaticCommand {
                continue;
            }
            if !seen.insert(skill.name.clone()) {
                continue;
            }
            skills.push(claude_core::plugins::types::Skill {
                name: skill.name.clone(),
                description: skill.description.clone(),
                content: skill.content.clone(),
                source: claude_core::plugins::types::SkillSource::Builtin,
                argument_hint: None,
                when_to_use: None,
                allowed_tools: Vec::new(),
                user_invocable: skill.user_invocable,
                disable_model_invocation: skill.disable_model_invocation,
                is_plugin_command: false,
            });
        }
    }
    if !skills.is_empty() {
        tracing::info!(count = skills.len(), "Discovered skills");
    }

    let configured_model = match cli.model.or_else(|| settings.model.clone()) {
        Some(model) => model,
        None => default_main_loop_model_setting().await,
    };
    let model = normalize_model_name(&configured_model);

    tracing::info!(
        "claude-rs initialized: model={}, tools={}, mcp_servers={}, skills={}, project={}",
        model,
        tools.all().len(),
        settings.mcp_servers.len(),
        skills.len(),
        project_root.display(),
    );

    // Build system prompt
    let tool_descriptions: Vec<(String, String)> = tools
        .all()
        .iter()
        .map(|t| (t.name().to_string(), t.description()))
        .collect();
    let system_prompt_values = claude_core::context::system_prompt::build_system_prompt(
        &project_root,
        &tool_descriptions,
        &model,
    )
    .await?;

    // Convert Vec<Value> to Vec<ContentBlock> for the engine
    let system_prompt: Vec<claude_core::types::content::ContentBlock> = system_prompt_values
        .into_iter()
        .filter_map(|v| {
            v.get("text").and_then(|t| t.as_str()).map(|text| {
                claude_core::types::content::ContentBlock::Text {
                    text: text.to_string(),
                }
            })
        })
        .collect();

    // Create API client
    let model_display = model.clone();
    let account_uuid = if matches!(&auth, claude_core::api::client::AuthMethod::OAuthToken(_)) {
        claude_core::config::global::load_global_config()
            .ok()
            .and_then(|config| config.oauth_account.map(|account| account.account_uuid))
            .unwrap_or_default()
    } else {
        String::new()
    };
    let api_config = claude_core::api::client::ApiConfig {
        max_tokens: resolve_max_output_tokens(&model, &settings),
        session_id: cli
            .session_id
            .clone()
            .unwrap_or_else(|| claude_core::api::client::get_session_id().clone()),
        account_uuid,
        effort: cli
            .effort
            .clone()
            .or_else(|| settings.effort_level.clone())
            .filter(|value| !value.trim().is_empty()),
        model: model.clone(),
        ..Default::default()
    };
    // Install a Haiku-backed secondary model so WebFetchTool and other tools
    // that need cheap post-processing can actually call the API. Without this
    // the prompt param on WebFetch is preserved in the result payload but no
    // summarisation happens.
    {
        let haiku = std::sync::Arc::new(claude_core::secondary_model::HaikuSecondaryModel::new(
            auth.clone(),
            api_config.base_url.clone(),
            api_config.session_id.clone(),
            model.clone(),
        ));
        claude_core::secondary_model::set_global(haiku);
    }

    // Install a HookRunner so TaskCreateTool (and future hook sites) can fire
    // user-configured hooks. We serialise the typed Settings to a JSON Value
    // so HookRunner::from_settings can pick the "hooks" subtree out of it.
    // If the user has no hooks configured the runner is a no-op.
    let hook_runner = {
        let cwd = std::env::current_dir()
            .ok()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let session_id = api_config.session_id.clone();
        let settings_value = raw_settings.clone();
        let runner = std::sync::Arc::new(claude_core::hooks::HookRunner::from_settings(
            &settings_value,
            cwd,
            session_id,
            String::new(),
        ));
        claude_core::hooks::set_global_runner(runner.clone());
        runner
    };

    let api_session_id = api_config.session_id.clone();
    let api_client = claude_core::api::client::ApiClient::new(api_config, auth.clone());

    // Create cancellation token
    let cancel = tokio_util::sync::CancellationToken::new();

    // Get tool definitions for the engine
    let tool_defs = tools.tool_definitions();

    // Create query engine
    let mut query_engine = claude_core::query::engine::QueryEngine::new(
        api_client,
        system_prompt,
        tool_defs,
        cancel.clone(),
    );
    if let Some(max) = cli.max_turns {
        query_engine.set_max_turns(max);
    }

    // Handle --resume: load transcript from a previous session.
    // Supports `--resume <session-id>` or `--resume last` to pick the most recent.
    if let Some(ref session_id_raw) = cli.resume {
        let resolved_id = if session_id_raw.eq_ignore_ascii_case("last") {
            // Find the most recent session by modification time
            let sessions = claude_core::session::manager::SessionManager::list_sessions()?;
            match sessions.first() {
                Some(info) => {
                    tracing::info!("Resolved --resume last to session '{}'", info.id);
                    info.id.clone()
                }
                None => {
                    eprintln!("No previous sessions found to resume.");
                    std::process::exit(1);
                }
            }
        } else {
            session_id_raw.clone()
        };

        let session_mgr = claude_core::session::manager::SessionManager::resume(&resolved_id)?;
        let transcript = session_mgr.storage().load_transcript()?;
        if transcript.is_empty() {
            eprintln!(
                "Warning: session '{}' has no transcript to resume",
                resolved_id
            );
        } else {
            tracing::info!(
                "Resuming session '{}' with {} messages",
                resolved_id,
                transcript.len()
            );
            eprintln!(
                "Resuming session {} ({} messages)",
                resolved_id,
                transcript.len()
            );
            query_engine.load_messages(transcript);
        }
    }

    let session_start = hook_runner
        .run_hooks(
            &claude_core::hooks::types::HookEvent::SessionStart,
            serde_json::json!({
                "source": if cli.resume.is_some() { "resume" } else { "startup" },
            }),
            None,
            None,
            None,
            None,
        )
        .await;
    if cli.output_format == OutputFormat::StreamJson {
        for result in &session_start.individual_results {
            emit_stream_json_hook_events(result, &api_session_id);
        }
    }
    for context in session_start.additional_contexts {
        query_engine.append_user_context_block(format!(
            "<system-reminder>\nSessionStart hook additional context: {}\n</system-reminder>",
            context
        ));
    }

    // Add MCP server instructions as request-time user context, matching TS.
    {
        let mgr = mcp_manager.read().await;
        let connections = mgr.connections().await;
        let mut instructions_parts: Vec<String> = Vec::new();
        let mut connected_instruction_blocks = Vec::new();
        for conn in &connections {
            if let claude_core::mcp::types::McpConnectionStatus::Connected {
                instructions: Some(ref instr),
                ..
            } = conn.status
            {
                connected_instruction_blocks
                    .push((conn.name.clone(), format!("## {}\n{}", conn.name, instr)));
            }
        }
        connected_instruction_blocks.sort_by(|a, b| a.0.cmp(&b.0));
        instructions_parts.extend(
            connected_instruction_blocks
                .into_iter()
                .map(|(_, block)| block),
        );
        if !instructions_parts.is_empty() {
            query_engine.append_user_context_block(format!(
                "<system-reminder>\n# MCP Server Instructions\n\nThe following MCP servers have provided instructions for how to use their tools and resources:\n\n{}\n</system-reminder>",
                instructions_parts.join("\n\n")
            ));
        }
    }

    // Add skill descriptions as request-time user context, matching TS.
    if !skills.is_empty() {
        let mut skills_text = String::from(
            "<system-reminder>\nThe following skills are available for use with the Skill tool:\n\n",
        );
        for skill in &skills {
            skills_text.push_str(&format!("- {}: {}", skill.name, skill.description));
            if let Some(ref hint) = skill.when_to_use {
                skills_text.push_str(&format!(" (use when: {})", hint));
            }
            skills_text.push('\n');
        }
        skills_text.push_str("</system-reminder>\n");
        query_engine.append_user_context_block(skills_text);
    }

    if let Some(context) =
        claude_core::context::system_prompt::build_project_user_context_block(&project_root)
    {
        query_engine.append_user_context_block(context);
    }

    // Append --append-system-prompt text (M2: was parsed but never applied)
    if let Some(ref extra) = cli.append_system_prompt {
        query_engine.append_system_prompt(extra.clone());
    }

    // Determine permission mode.
    // Priority: CLI flag > CLAUDE_PERMISSION_MODE env var > Default.
    // The env var is set by agent_tool.rs when spawning sub-agents to
    // propagate the parent's permission mode.
    let permission_mode = if cli.dangerously_skip_permissions {
        claude_core::permissions::types::PermissionMode::BypassPermissions
    } else if let Some(mode_str) = &cli.permission_mode {
        claude_core::permissions::types::PermissionMode::from_string(mode_str)
    } else if let Ok(mode_str) = std::env::var("CLAUDE_PERMISSION_MODE") {
        claude_core::permissions::types::PermissionMode::from_string(&mode_str)
    } else if let Some(mode) =
        claude_core::permissions::permission_mode_from_settings_value(&permission_settings)
    {
        mode
    } else {
        claude_core::permissions::types::PermissionMode::Default
    };
    let permission_rules_from_disk =
        claude_core::permissions::load_permission_rules_from_disk_by_source(&project_root);
    let additional_dirs =
        claude_core::permissions::permission_additional_directories_from_settings_value(
            &permission_settings,
        );
    let mut disallowed_tools_cli = cli.disallowed_tools.clone();
    disallowed_tools_cli.extend(base_tool_denials_from_cli_tools(
        &cli.tools,
        &base_permission_tool_names,
    ));
    let mut add_dirs = additional_dirs;
    add_dirs.extend(cli.add_dirs.clone());
    let initial_permission_context = claude_core::permissions::initialize_tool_permission_context(
        &cli.allowed_tools,
        &disallowed_tools_cli,
        permission_mode.clone(),
        cli.dangerously_skip_permissions,
        &add_dirs,
        &permission_rules_from_disk,
        cwd.clone(),
    )
    .tool_permission_context;

    // Handle non-interactive prompt mode
    if let Some(prompt) = prompt_arg {
        // If using OAuth proxy, delegate to real claude binary
        use claude_core::hooks::{
            get_global_runner, resolve_hook_permission_decision, run_post_tool_use_failure_hooks,
            run_post_tool_use_hooks, run_pre_tool_use_hooks, ResolvedPermission,
        };
        use claude_core::permissions::evaluator::{evaluate_permission, SimpleToolPermissions};
        use claude_core::permissions::types::PermissionDecision;
        use claude_core::query::engine::TurnResult;
        use claude_core::types::events::StreamEvent;
        use claude_tools::ToolUseContext;
        use std::path::PathBuf;
        use tokio::sync::mpsc;

        let mut cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        // Remember the original directory so ExitWorktree can restore it.
        let original_cwd = cwd.clone();
        let read_file_state = std::sync::Arc::new(std::sync::Mutex::new(
            claude_tools::registry::ReadFileState::new(),
        ));
        let mut perm_ctx = initial_permission_context.clone();

        // Check if prompt is a skill invocation (e.g. "/commit fix typo")
        let mut effective_prompt = prompt.clone();
        for skill in &skills {
            if let Some(args) = claude_core::plugins::skill::match_skill(&prompt, skill) {
                let mut content = skill.content.clone();
                if !args.is_empty() {
                    content.push_str(&format!("\n\nArguments: {}", args));
                }
                effective_prompt = content;
                break;
            }
        }

        if cli.output_format == OutputFormat::StreamJson {
            let (mcp_servers, mcp_prompt_commands) = {
                let mgr = mcp_manager.read().await;
                (
                    stream_json_mcp_servers_in_order(mgr.connections().await, &mcp_server_order),
                    mgr.prompt_command_names_in_order(&mcp_server_order).await,
                )
            };
            emit_stream_json(stream_json_init_event(StreamJsonInitMeta {
                cwd: &cwd,
                session_id: &api_session_id,
                tool_names: tools
                    .all()
                    .iter()
                    .map(|tool| {
                        if tool.name() == "Agent" {
                            "Task".to_string()
                        } else {
                            tool.name().to_string()
                        }
                    })
                    .collect(),
                mcp_servers,
                model_display: &model_display,
                permission_mode: &permission_mode,
                registered_skills: &registered_skills,
                discovered_skills: &discovered_skills,
                stream_skill_names: &stream_skill_names,
                mcp_prompt_commands: &mcp_prompt_commands,
                output_style: settings.output_style.as_deref(),
                auth: &auth,
            }));
        }

        query_engine.add_user_message(&effective_prompt);

        // Run the agentic loop: prompt -> run_turn -> ToolUse* -> Done
        let mut final_text = String::new();
        let session_id = api_session_id.clone();
        let started_at = std::time::Instant::now();
        let mut latest_usage: Option<claude_core::types::usage::Usage> = None;
        let mut num_turns: u32 = 0;
        let final_stop_reason = loop {
            let (stream_tx, mut stream_rx) = mpsc::channel::<StreamEvent>(128);
            let output_format = cli.output_format.clone();
            let stream_session_id = session_id.clone();

            // Spawn a task to print streamed text to stdout
            let print_handle = tokio::spawn(async move {
                let mut state = StreamJsonPrintState::default();
                while let Some(ev) = stream_rx.recv().await {
                    match &output_format {
                        OutputFormat::Text => match ev {
                            StreamEvent::TextDelta { text } => {
                                print!("{}", text);
                                use std::io::Write;
                                let _ = std::io::stdout().flush();
                            }
                            StreamEvent::Done { .. } => {
                                println!();
                            }
                            _ => {}
                        },
                        OutputFormat::Json => {
                            if let StreamEvent::TextDelta { text: delta } = ev {
                                state.text.push_str(&delta);
                            }
                        }
                        OutputFormat::StreamJson => {
                            if let StreamEvent::TextDelta { text: delta } = &ev {
                                state.text.push_str(delta);
                            }
                            if let StreamEvent::UsageUpdate(usage) = &ev {
                                merge_stream_usage(&mut state.latest_usage, usage.clone());
                            }
                            if let Some(value) =
                                stream_event_to_stream_json(&ev, &stream_session_id)
                            {
                                emit_stream_json(value);
                            }
                        }
                    }
                }
                state
            });

            num_turns += 1;
            let result = query_engine.run_turn(&stream_tx).await?;
            drop(stream_tx);
            if let Ok(state) = print_handle.await {
                final_text.push_str(&state.text);
                if state.latest_usage.is_some() {
                    latest_usage = state.latest_usage;
                }
            }

            match result {
                TurnResult::Done(stop_reason) => {
                    break serde_json::to_string(&stop_reason)
                        .ok()
                        .and_then(|value| serde_json::from_str::<String>(&value).ok())
                        .unwrap_or_else(|| "end_turn".to_string());
                }
                TurnResult::ContinueRecovery => {
                    // max_tokens recovery — run again immediately
                    continue;
                }
                TurnResult::ToolUse(tool_uses) => {
                    // Execute each tool, check permissions, feed results back
                    let mut stream_tool_results = Vec::new();
                    for tool_info in &tool_uses {
                        let mut tool_input = tool_info.input.clone();
                        let mut forced_permission: Option<Result<(), String>> = None;
                        if let Some(runner) = get_global_runner() {
                            let pre = run_pre_tool_use_hooks(
                                &runner,
                                &tool_info.name,
                                &tool_info.id,
                                &tool_input,
                                Some(permission_mode_hook_name(&permission_mode)),
                                None,
                                None,
                            )
                            .await;
                            for context in &pre.additional_contexts {
                                query_engine.append_user_context_block(context.clone());
                            }
                            if let Some(message) = hook_blocking_errors_text(&pre.blocking_errors)
                                .or_else(|| pre.denial_message.clone())
                            {
                                forced_permission = Some(Err(message));
                            } else if pre.prevent_continuation {
                                forced_permission =
                                    Some(Err(pre.stop_reason.unwrap_or_else(|| {
                                        "PreToolUse hook stopped tool execution".to_string()
                                    })));
                            } else {
                                let resolved = resolve_hook_permission_decision(
                                    &pre,
                                    &tool_input,
                                    |candidate_input| {
                                        let tool_name = tool_info.name.clone();
                                        let perm_ctx = perm_ctx.clone();
                                        let tools = tools.clone();
                                        let candidate_input = candidate_input.clone();
                                        async move {
                                            let decision = if let Some(tool) =
                                                tools.get(&tool_name)
                                            {
                                                let tool_perms =
                                                    claude_tools::registry::ExecutorToolPermissions::new(
                                                        tool,
                                                        candidate_input.clone(),
                                                    );
                                                evaluate_permission(
                                                    &tool_perms,
                                                    &candidate_input,
                                                    &perm_ctx,
                                                )
                                            } else {
                                                let tool_perms = SimpleToolPermissions::new(
                                                    &tool_name,
                                                    false,
                                                );
                                                evaluate_permission(
                                                    &tool_perms,
                                                    &candidate_input,
                                                    &perm_ctx,
                                                )
                                            };
                                            permission_decision_to_rule_check(&decision)
                                        }
                                    },
                                )
                                .await;
                                match resolved {
                                    ResolvedPermission::Allow { updated_input }
                                    | ResolvedPermission::NormalFlow { updated_input } => {
                                        tool_input =
                                            merge_hook_updated_input(&tool_input, &updated_input);
                                    }
                                    ResolvedPermission::Deny { message } => {
                                        forced_permission =
                                            Some(Err(message.unwrap_or_else(|| {
                                                "PreToolUse hook denied this tool".to_string()
                                            })));
                                    }
                                    ResolvedPermission::RequiresUserConfirmation {
                                        updated_input,
                                        force_decision,
                                    } => {
                                        tool_input =
                                            merge_hook_updated_input(&tool_input, &updated_input);
                                        forced_permission = Some(Err(force_decision
                                            .unwrap_or_else(|| {
                                                "Tool requires user confirmation".to_string()
                                            })));
                                    }
                                }
                            }
                        }

                        let decision = if let Some(forced) = forced_permission {
                            match forced {
                                Ok(()) => PermissionDecision::allow(),
                                Err(message) => PermissionDecision::deny(
                                    message,
                                    claude_core::permissions::types::PermissionDecisionReason::Hook {
                                        hook_name: format!("PreToolUse:{}", tool_info.name),
                                        hook_source: None,
                                        reason: None,
                                    },
                                ),
                            }
                        } else {
                            if let Some(tool) = tools.get(&tool_info.name) {
                                let tool_perms =
                                    claude_tools::registry::ExecutorToolPermissions::new(
                                        tool,
                                        tool_input.clone(),
                                    );
                                evaluate_permission(&tool_perms, &tool_input, &perm_ctx)
                            } else {
                                let tool_perms = SimpleToolPermissions::new(&tool_info.name, false);
                                evaluate_permission(&tool_perms, &tool_input, &perm_ctx)
                            }
                        };

                        let (mut result_text, mut is_error, mut result_json) = match decision {
                            PermissionDecision::Ask(ask) => {
                                if let Some(permission_prompt_tool) = &permission_prompt_tool {
                                    match run_permission_prompt_tool(
                                        permission_prompt_tool,
                                        &tool_info.name,
                                        &tool_info.id,
                                        &tool_input,
                                        &cwd,
                                        read_file_state.clone(),
                                        permission_mode.clone(),
                                        &model,
                                        cancel.clone(),
                                    )
                                    .await
                                    {
                                        Ok(output) if output.behavior == "allow" => {
                                            if let Some(updates) = output.updated_permissions {
                                                perm_ctx = claude_core::permissions::evaluator::apply_permission_updates(
                                                    perm_ctx,
                                                    &updates,
                                                );
                                                if let Err(err) = claude_core::permissions::evaluator::persist_permission_updates(
                                                    &updates,
                                                    &cwd,
                                                ) {
                                                    tracing::warn!(
                                                        error = %err,
                                                        "failed to persist permission prompt tool updates"
                                                    );
                                                }
                                            }
                                            let updated_input = output
                                                .updated_input
                                                .unwrap_or_else(|| tool_input.clone());
                                            tool_input = if updated_input
                                                .as_object()
                                                .is_some_and(|obj| obj.is_empty())
                                            {
                                                tool_input
                                            } else {
                                                updated_input
                                            };
                                            let executor = tools.get(&tool_info.name);
                                            match executor {
                                                Some(exec) => {
                                                    let ctx = ToolUseContext::new(
                                                        cwd.clone(),
                                                        read_file_state.clone(),
                                                        permission_mode.clone(),
                                                        std::sync::Arc::new(
                                                            claude_core::tool_use_context_options::ToolUseContextOptions::minimal(&model),
                                                        ),
                                                        std::sync::Arc::new(claude_core::tool_host::NullToolHost),
                                                    );
                                                    match exec
                                                        .call(
                                                            &tool_input,
                                                            &ctx,
                                                            cancel.clone(),
                                                            None,
                                                        )
                                                        .await
                                                    {
                                                        Ok(data) => {
                                                            if !data.is_error {
                                                                match tool_info.name.as_str() {
                                                                    "EnterWorktree" => {
                                                                        if let Some(path) = data
                                                                            .data["worktreePath"]
                                                                            .as_str()
                                                                        {
                                                                            cwd =
                                                                                PathBuf::from(path);
                                                                            tracing::info!(
                                                                                "Session cwd switched to worktree: {}",
                                                                                path
                                                                            );
                                                                        }
                                                                    }
                                                                    "ExitWorktree" => {
                                                                        cwd = original_cwd.clone();
                                                                        tracing::info!(
                                                                            "Session cwd restored to: {}",
                                                                            original_cwd.display()
                                                                        );
                                                                    }
                                                                    _ => {}
                                                                }
                                                            }
                                                            let text = format_tool_result_for_model(
                                                                &tool_info.name,
                                                                &data.data,
                                                            );
                                                            (text, data.is_error, data.data)
                                                        }
                                                        Err(e) => {
                                                            let message = format!("Error: {}", e);
                                                            (
                                                                message.clone(),
                                                                true,
                                                                serde_json::json!({"error": message}),
                                                            )
                                                        }
                                                    }
                                                }
                                                None => {
                                                    let message =
                                                        format!("Unknown tool: {}", tool_info.name);
                                                    (
                                                        message.clone(),
                                                        true,
                                                        serde_json::json!({"error": message}),
                                                    )
                                                }
                                            }
                                        }
                                        Ok(output) => {
                                            let message = output.message.unwrap_or_else(|| {
                                                "Permission denied by permission prompt tool"
                                                    .to_string()
                                            });
                                            (
                                                format!("Permission denied: {}", message),
                                                true,
                                                serde_json::json!({"error": message}),
                                            )
                                        }
                                        Err(err) => {
                                            let message = err.to_string();
                                            (
                                                format!(
                                                    "Permission prompt tool error: {}",
                                                    message
                                                ),
                                                true,
                                                serde_json::json!({"error": message}),
                                            )
                                        }
                                    }
                                } else {
                                    // In non-interactive / headless mode, Ask decisions are DENIED
                                    // (matching TS headless behavior). Auto-allowing would bypass
                                    // permission semantics when running unattended.
                                    tracing::warn!(
                                        tool = %tool_info.name,
                                        reason = %ask.message,
                                        "Non-interactive mode: denying tool requiring user confirmation"
                                    );
                                    (
                                        format!(
                                            "Permission denied (non-interactive): {}",
                                            ask.message
                                        ),
                                        true,
                                        serde_json::json!({"error": ask.message}),
                                    )
                                }
                            }
                            PermissionDecision::Allow(_) => {
                                let executor = tools.get(&tool_info.name);
                                match executor {
                                    Some(exec) => {
                                        // Explicit construction (not `for_test`) so the session's
                                        // live `model` reaches `ctx.options.main_loop_model` —
                                        // consumed by e.g. the command adapter at
                                        // `claude-core/src/command_adapter.rs:114`.
                                        let ctx = ToolUseContext::new(
                                            cwd.clone(),
                                            read_file_state.clone(),
                                            permission_mode.clone(),
                                            std::sync::Arc::new(
                                                claude_core::tool_use_context_options::ToolUseContextOptions::minimal(&model),
                                            ),
                                            std::sync::Arc::new(claude_core::tool_host::NullToolHost),
                                        );
                                        match exec
                                            .call(&tool_input, &ctx, cancel.clone(), None)
                                            .await
                                        {
                                            Ok(data) => {
                                                // Update cwd when entering/exiting a worktree.
                                                if !data.is_error {
                                                    match tool_info.name.as_str() {
                                                        "EnterWorktree" => {
                                                            if let Some(path) =
                                                                data.data["worktreePath"].as_str()
                                                            {
                                                                cwd = PathBuf::from(path);
                                                                tracing::info!(
                                                                    "Session cwd switched to worktree: {}",
                                                                    path
                                                                );
                                                            }
                                                        }
                                                        "ExitWorktree" => {
                                                            cwd = original_cwd.clone();
                                                            tracing::info!(
                                                                "Session cwd restored to: {}",
                                                                original_cwd.display()
                                                            );
                                                        }
                                                        _ => {}
                                                    }
                                                }
                                                let text = format_tool_result_for_model(
                                                    &tool_info.name,
                                                    &data.data,
                                                );
                                                (text, data.is_error, data.data)
                                            }
                                            Err(e) => {
                                                let message = format!("Error: {}", e);
                                                (
                                                    message.clone(),
                                                    true,
                                                    serde_json::json!({"error": message}),
                                                )
                                            }
                                        }
                                    }
                                    None => {
                                        let message = format!("Unknown tool: {}", tool_info.name);
                                        (
                                            message.clone(),
                                            true,
                                            serde_json::json!({"error": message}),
                                        )
                                    }
                                }
                            }
                            PermissionDecision::Deny(deny) => (
                                format!("Permission denied: {}", deny.message),
                                true,
                                serde_json::json!({"error": deny.message}),
                            ),
                        };

                        if let Some(runner) = get_global_runner() {
                            if is_error {
                                let failure = run_post_tool_use_failure_hooks(
                                    &runner,
                                    &tool_info.name,
                                    &tool_info.id,
                                    &tool_input,
                                    &result_text,
                                    None,
                                    Some(permission_mode_hook_name(&permission_mode)),
                                    None,
                                    None,
                                )
                                .await;
                                for context in &failure.additional_contexts {
                                    query_engine.append_user_context_block(context.clone());
                                }
                                if let Some(message) =
                                    hook_blocking_errors_text(&failure.blocking_errors)
                                {
                                    result_text = message;
                                    is_error = true;
                                }
                            } else {
                                let post = run_post_tool_use_hooks(
                                    &runner,
                                    &tool_info.name,
                                    &tool_info.id,
                                    &tool_input,
                                    &result_json,
                                    Some(permission_mode_hook_name(&permission_mode)),
                                    None,
                                    None,
                                )
                                .await;
                                for context in &post.additional_contexts {
                                    query_engine.append_user_context_block(context.clone());
                                }
                                if let Some(updated) = post.updated_mcp_tool_output {
                                    result_json = updated;
                                    result_text = result_json
                                        .as_str()
                                        .unwrap_or(&result_json.to_string())
                                        .to_string();
                                }
                                if let Some(message) =
                                    hook_blocking_errors_text(&post.blocking_errors)
                                {
                                    result_text = message;
                                    is_error = true;
                                } else if post.prevent_continuation {
                                    result_text = post.stop_reason.unwrap_or_else(|| {
                                        "PostToolUse hook stopped continuation".to_string()
                                    });
                                    is_error = true;
                                }
                            }
                        }

                        let result_content = if is_error {
                            serde_json::Value::String(result_text.clone())
                        } else {
                            format_tool_result_content_for_model(&tool_info.name, &result_json)
                        };

                        if cli.output_format == OutputFormat::StreamJson {
                            stream_tool_results.push(serde_json::json!({
                                "type": "tool_result",
                                "tool_use_id": tool_info.id,
                                "content": result_content.clone(),
                                "is_error": is_error,
                            }));
                        }
                        query_engine.add_tool_result_content_with_error_field(
                            &tool_info.id,
                            result_content,
                            is_error,
                            is_error || tool_info.name == "Bash",
                        );
                    }
                    if cli.output_format == OutputFormat::StreamJson
                        && !stream_tool_results.is_empty()
                    {
                        emit_stream_json(stream_json_user_tool_result_event(
                            stream_tool_results,
                            &session_id,
                        ));
                    }
                    // Continue the loop to call run_turn again with the tool results
                }
            }
        };

        if cli.output_format == OutputFormat::Json {
            println!(
                "{}",
                serde_json::to_string(&serde_json::json!({
                    "type": "result",
                    "subtype": "success",
                    "is_error": false,
                    "result": final_text,
                }))?
            );
        } else if cli.output_format == OutputFormat::StreamJson {
            let mut cost_tracker = claude_core::cost::tracker::CostTracker::new(&model);
            if let Some(ref usage) = latest_usage {
                cost_tracker.add_usage(usage);
            }
            let context_window = if model_display.contains("[1M]") || model_display.contains("[1m]")
            {
                1_000_000
            } else {
                claude_core::compact::compactor::default_context_window()
            };
            emit_stream_json(stream_json_result_event_with_meta(
                &final_text,
                &session_id,
                StreamJsonResultMeta {
                    duration_ms: started_at.elapsed().as_millis(),
                    num_turns,
                    stop_reason: &final_stop_reason,
                    usage: latest_usage.as_ref(),
                    model_display: &model_display,
                    max_tokens: resolve_max_output_tokens(&model, &settings),
                    context_window,
                    total_cost_usd: cost_tracker.total_cost_usd(),
                },
            ));
        }

        // Gracefully disconnect MCP servers
        let mgr = mcp_manager.read().await;
        mgr.disconnect_all().await;

        return Ok(());
    }

    // Interactive TUI mode
    let mut app = claude_tui::app::App::new()?;
    app.set_model_name(&model_display);
    app.set_skills(skills);
    app.run_with_engine(
        query_engine,
        tools,
        cancel.clone(),
        initial_permission_context,
        api_session_id.clone(),
    )
    .await?;

    // Gracefully disconnect MCP servers
    let mgr = mcp_manager.read().await;
    mgr.disconnect_all().await;

    Ok(())
}
