use claude_tools::build_default_registry;

#[test]
fn test_default_registry_has_all_phase1_tools() {
    let reg = build_default_registry();
    assert!(reg.get("Bash").is_some());
    assert!(reg.get("Read").is_some());
    assert!(reg.get("Write").is_some());
    assert!(reg.get("Edit").is_some());
    assert!(reg.get("Grep").is_some());
    assert!(reg.get("Glob").is_some());
    assert!(reg.get("LSP").is_some());
}

#[test]
fn test_default_registry_schemas() {
    let reg = build_default_registry();
    let schemas = reg.schemas();
    assert!(
        schemas.len() >= 27,
        "expected at least 27 tools, got {}",
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
    ];
    for name in &hidden {
        assert!(
            reg.get(name).is_none(),
            "{name} should be hidden by default — is its feature gate wired?"
        );
    }
}
