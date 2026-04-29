use claude_core::config::settings::PermissionRuleConfig;
use claude_core::permissions::{PermissionMode, PermissionRuleSource, ToolPermissionContext};
use claude_tools::{
    build_default_registry, build_default_registry_with_options, filter_registry_by_deny_rules,
    filter_registry_by_permission_context, RegistryOptions,
};
use std::collections::HashMap;
use std::sync::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn clear_registry_env() {
    for key in [
        "CLAUDE_CODE_SIMPLE",
        "USER_TYPE",
        "CLAUDE_REPL_MODE",
        "CLAUDE_CODE_REPL",
        "CLAUDE_CODE_ENTRYPOINT",
        "MONITOR_TOOL",
        "HISTORY_SNIP",
        "CONTEXT_COLLAPSE",
        "TERMINAL_PANEL",
        "WEB_BROWSER_TOOL",
        "UDS_INBOX",
        "WORKFLOW_SCRIPTS",
        "CLAUDE_CODE_VERIFY_PLAN",
        "AGENT_TRIGGERS",
        "AGENT_TRIGGERS_REMOTE",
        "KAIROS",
        "KAIROS_PUSH_NOTIFICATION",
        "KAIROS_GITHUB_WEBHOOKS",
        "PROACTIVE",
        "ENABLE_LSP_TOOL",
        "CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS",
        "CLAUDE_CODE_USE_POWERSHELL_TOOL",
        "CLAUDE_CODE_ENABLE_TASKS",
        "ANTHROPIC_BASE_URL",
        "ENABLE_TOOL_SEARCH",
        "CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS",
        "CLAUDE_CODE_USE_BEDROCK",
        "CLAUDE_CODE_USE_VERTEX",
        "CLAUDE_CODE_USE_FOUNDRY",
    ] {
        std::env::remove_var(key);
    }
}

#[test]
fn test_default_registry_has_all_phase1_tools() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    clear_registry_env();

    let reg = build_default_registry();
    assert!(reg.get("Bash").is_some());
    assert!(reg.get("Read").is_some());
    assert!(reg.get("Write").is_some());
    assert!(reg.get("Edit").is_some());
    assert!(reg.get("Grep").is_some());
    assert!(reg.get("Glob").is_some());
    assert!(reg.get("CronCreate").is_some());
    assert!(reg.get("CronDelete").is_some());
    assert!(reg.get("CronList").is_some());
    assert!(reg.get("LSP").is_some());
    assert!(reg.get("Monitor").is_some());
    assert!(reg.get("PushNotification").is_some());
    assert!(reg.get("RemoteTrigger").is_some());
    assert!(reg.get("ScheduleWakeup").is_some());
    assert!(reg.get("Brief").is_none());
    assert!(reg.get("SendMessage").is_none());
    assert!(
        reg.get("TodoWrite").is_none(),
        "TodoWrite is hidden when interactive Task v2 tools are enabled"
    );
    assert!(reg.get("TaskCreate").is_some());
    assert!(
        reg.get("StructuredOutput").is_none(),
        "StructuredOutput is synthetic and should only be injected for structured-output requests"
    );
}

#[test]
fn test_default_registry_schemas() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    clear_registry_env();

    let reg = build_default_registry();
    let schemas = reg.schemas();
    assert!(
        schemas.len() >= 20,
        "expected at least 20 tools, got {}",
        schemas.len()
    );
    for schema in &schemas {
        assert!(schema.get("name").is_some());
        assert!(schema.get("input_schema").is_some());
    }
}

/// Tools that mirror TS `feature('X')` gates or `USER_TYPE=ant` checks must
/// follow the installed TS build defaults. Several bun `feature(...)` gates
/// are enabled in the reference build even when no shell env var is set.
#[test]
fn test_gated_tools_hidden_by_default() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    clear_registry_env();

    // These env vars must be unset for this test to be meaningful.
    // Other tests in the same process can set them — guard explicitly.
    let gates = [
        "MONITOR_TOOL",
        "HISTORY_SNIP",
        "CONTEXT_COLLAPSE",
        "TERMINAL_PANEL",
        "WEB_BROWSER_TOOL",
        "UDS_INBOX",
        "WORKFLOW_SCRIPTS",
        "CLAUDE_CODE_VERIFY_PLAN",
        "KAIROS",
        "KAIROS_GITHUB_WEBHOOKS",
        "CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS",
        "CLAUDE_CODE_USE_POWERSHELL_TOOL",
        "USER_TYPE",
    ];
    for g in &gates {
        if std::env::var(g).is_ok() {
            eprintln!("skipping gated-hidden assertion: {g} is set in env");
            return;
        }
    }

    let reg = build_default_registry();
    let hidden = [
        "Snip",
        "CtxInspect",
        "TerminalCapture",
        "WebBrowser",
        "ListPeers",
        "Workflow",
        "VerifyPlanExecution",
        "SendUserFile",
        "SubscribePR",
        "Config",
        "REPL",
        "SuggestBackgroundPR",
        "TeamCreate",
        "TeamDelete",
        "PowerShell",
        "SendMessage",
        "Brief",
    ];
    for name in &hidden {
        assert!(
            reg.get(name).is_none(),
            "{name} should be hidden by default — is its feature gate wired?"
        );
    }
}

#[test]
fn test_non_interactive_uses_todowrite_unless_tasks_are_forced() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    clear_registry_env();

    if std::env::var("CLAUDE_CODE_ENABLE_TASKS").is_ok() {
        eprintln!("skipping non-interactive task assertion: CLAUDE_CODE_ENABLE_TASKS is set");
        return;
    }

    let reg = build_default_registry_with_options(RegistryOptions {
        is_non_interactive_session: true,
    });
    assert!(reg.get("TodoWrite").is_some());
    assert!(reg.get("TaskCreate").is_none());
    assert!(reg.get("TaskList").is_none());
    assert!(reg.get("TaskUpdate").is_none());
    assert!(reg.get("TaskGet").is_none());
    assert!(reg.get("TaskStop").is_some());
    assert!(reg.get("TaskOutput").is_some());
}

#[test]
fn test_simple_mode_matches_ts_tool_subset() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    clear_registry_env();
    std::env::set_var("CLAUDE_CODE_SIMPLE", "1");

    let reg = build_default_registry();
    let names = reg
        .all()
        .iter()
        .map(|tool| tool.name().to_string())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["Bash", "Read", "Edit"]);

    std::env::remove_var("CLAUDE_CODE_SIMPLE");
}

#[test]
fn test_repl_mode_hides_repl_only_tools_when_repl_is_available() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    clear_registry_env();
    std::env::set_var("USER_TYPE", "ant");
    std::env::set_var("CLAUDE_REPL_MODE", "1");

    let reg = build_default_registry();
    assert!(reg.get("REPL").is_some());
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
        assert!(
            reg.get(name).is_none(),
            "{name} should be hidden by REPL mode"
        );
    }
    assert!(reg.get("TaskStop").is_some());
    assert!(reg.get("SendMessage").is_some());

    std::env::remove_var("USER_TYPE");
    std::env::remove_var("CLAUDE_REPL_MODE");
}

#[test]
fn test_tool_search_hidden_by_default_for_non_first_party_base_url() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    clear_registry_env();
    std::env::set_var("ANTHROPIC_BASE_URL", "http://127.0.0.1:8787");

    let reg = build_default_registry();
    assert!(reg.get("ToolSearch").is_none());

    std::env::set_var("ENABLE_TOOL_SEARCH", "true");
    let reg = build_default_registry();
    assert!(reg.get("ToolSearch").is_some());

    clear_registry_env();
}

#[test]
fn test_blanket_deny_rules_filter_default_registry() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    clear_registry_env();

    let mut reg = build_default_registry();
    filter_registry_by_deny_rules(
        &mut reg,
        &[
            PermissionRuleConfig {
                tool: "Read".to_string(),
                pattern: None,
            },
            PermissionRuleConfig {
                tool: "Bash".to_string(),
                pattern: Some("git status".to_string()),
            },
            PermissionRuleConfig {
                tool: "ToolSearch".to_string(),
                pattern: Some("*".to_string()),
            },
        ],
    );

    assert!(reg.get("Read").is_none());
    assert!(
        reg.get("Bash").is_some(),
        "content-specific deny rules must not hide the whole tool"
    );
    assert!(reg.get("ToolSearch").is_none());
}

#[test]
fn test_permission_context_deny_rules_filter_registry_like_ts() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    clear_registry_env();
    std::env::set_var("ENABLE_TOOL_SEARCH", "true");

    let mut deny = HashMap::new();
    deny.insert(
        PermissionRuleSource::CliArg,
        vec![
            "Read".to_string(),
            "Bash(git status)".to_string(),
            "ToolSearch".to_string(),
        ],
    );
    let ctx = ToolPermissionContext {
        mode: PermissionMode::Default,
        always_deny_rules: deny,
        ..Default::default()
    };

    let mut reg = build_default_registry();
    filter_registry_by_permission_context(&mut reg, &ctx);
    claude_tools::register_tool_search_snapshot(&mut reg);
    filter_registry_by_permission_context(&mut reg, &ctx);

    assert!(reg.get("Read").is_none());
    assert!(
        reg.get("Bash").is_some(),
        "content-specific deny rules must not hide the whole tool"
    );
    assert!(
        reg.get("ToolSearch").is_none(),
        "ToolSearch must not be reintroduced after a deny-rule filter"
    );

    std::env::remove_var("ENABLE_TOOL_SEARCH");
}

#[test]
fn test_permission_context_mcp_server_deny_filters_all_server_tools_like_ts() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    clear_registry_env();

    let mut deny = HashMap::new();
    deny.insert(
        PermissionRuleSource::CliArg,
        vec!["mcp__jira".to_string(), "mcp__slack__post".to_string()],
    );
    let ctx = ToolPermissionContext {
        mode: PermissionMode::Default,
        always_deny_rules: deny,
        ..Default::default()
    };

    let mut reg = claude_tools::ToolRegistry::new();
    reg.register(std::sync::Arc::new(claude_tools::mcp_tool::McpTool::new(
        "mcp__jira__search".to_string(),
        "search".to_string(),
        "jira".to_string(),
        "Search Jira".to_string(),
        serde_json::json!({"type": "object"}),
        std::sync::Arc::new(tokio::sync::RwLock::new(
            claude_core::mcp::manager::McpManager::new(),
        )),
    )));
    reg.register(std::sync::Arc::new(claude_tools::mcp_tool::McpTool::new(
        "mcp__slack__post".to_string(),
        "post".to_string(),
        "slack".to_string(),
        "Post Slack".to_string(),
        serde_json::json!({"type": "object"}),
        std::sync::Arc::new(tokio::sync::RwLock::new(
            claude_core::mcp::manager::McpManager::new(),
        )),
    )));
    reg.register(std::sync::Arc::new(claude_tools::mcp_tool::McpTool::new(
        "mcp__slack__search".to_string(),
        "search".to_string(),
        "slack".to_string(),
        "Search Slack".to_string(),
        serde_json::json!({"type": "object"}),
        std::sync::Arc::new(tokio::sync::RwLock::new(
            claude_core::mcp::manager::McpManager::new(),
        )),
    )));

    filter_registry_by_permission_context(&mut reg, &ctx);

    assert!(reg.get("mcp__jira__search").is_none());
    assert!(reg.get("mcp__slack__post").is_none());
    assert!(reg.get("mcp__slack__search").is_some());
}
