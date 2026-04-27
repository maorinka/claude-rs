use claude_core::config::settings::PermissionRuleConfig;
use claude_tools::{
    build_default_registry, build_default_registry_with_options, filter_registry_by_deny_rules,
    RegistryOptions,
};

#[test]
fn test_default_registry_has_all_phase1_tools() {
    let reg = build_default_registry();
    assert!(reg.get("Bash").is_some());
    assert!(reg.get("Read").is_some());
    assert!(reg.get("Write").is_some());
    assert!(reg.get("Edit").is_some());
    assert!(reg.get("Grep").is_some());
    assert!(reg.get("Glob").is_some());
    assert!(reg.get("Brief").is_some());
    assert!(reg.get("SendMessage").is_some());
    assert!(
        reg.get("LSP").is_none(),
        "LSP is hidden unless ENABLE_LSP_TOOL is truthy"
    );
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
/// NOT be registered when their env var is unset. Catches accidental
/// un-gating during refactors. Env vars are left unset in the CI test
/// harness, so this is the default state.
#[test]
fn test_gated_tools_hidden_by_default() {
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
        "AGENT_TRIGGERS",
        "AGENT_TRIGGERS_REMOTE",
        "KAIROS",
        "KAIROS_PUSH_NOTIFICATION",
        "KAIROS_GITHUB_WEBHOOKS",
        "PROACTIVE",
        "ENABLE_LSP_TOOL",
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
        "Monitor",
        "Snip",
        "CtxInspect",
        "TerminalCapture",
        "WebBrowser",
        "ListPeers",
        "Workflow",
        "VerifyPlanExecution",
        "ScheduleCron", // alias — both paths must resolve to None when gate off
        "CronCreate",   // canonical name (matches TS)
        "CronDelete",
        "CronList",
        "RemoteTrigger",
        "PushNotification",
        "SendUserFile",
        "SubscribePR",
        "Config",
        "REPL",
        "SuggestBackgroundPR",
        "LSP",
        "TeamCreate",
        "TeamDelete",
        "Sleep",
        "PowerShell",
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
fn test_blanket_deny_rules_filter_default_registry() {
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
