use claude_core::commands::builtin::build_default_commands;
use claude_core::commands::registry::{
    Command, CommandContext, CommandHandler, CommandRegistry, CommandResult, CommandType,
    SharedCommandState,
};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use anyhow::Result;

fn make_ctx() -> CommandContext {
    CommandContext {
        working_directory: PathBuf::from("/tmp"),
        model: "claude-opus-4".to_string(),
        shared: None,
    }
}

fn make_ctx_with_shared() -> (CommandContext, Arc<Mutex<SharedCommandState>>) {
    let shared = Arc::new(Mutex::new(SharedCommandState {
        model: "claude-opus-4".to_string(),
        total_tokens: 50000,
        message_count: 12,
        session_id: "abc-1234-def".to_string(),
        permission_mode: "default".to_string(),
        cost_summary: "Tokens: 30000 in / 20000 out | Cache: 0 read / 0 write | Requests: 5 | Cost: $0.3750".to_string(),
        request_count: 5,
        total_cost_usd: 0.375,
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
        working_directory: PathBuf::from("/tmp"),
        model: "claude-opus-4".to_string(),
        shared: Some(shared.clone()),
    };
    (ctx, shared)
}

// ---------------------------------------------------------------------------
// test_register_and_get_command
// ---------------------------------------------------------------------------
#[test]
fn test_register_and_get_command() {
    struct EchoHandler;
    impl CommandHandler for EchoHandler {
        fn execute(&self, args: &str, _ctx: &CommandContext) -> Result<CommandResult> {
            Ok(CommandResult::Action(format!("echo: {}", args)))
        }
    }

    let mut registry = CommandRegistry::new();
    registry.register(Command {
        name: "echo".to_string(),
        description: "Echo arguments".to_string(),
        command_type: CommandType::Action,
        handler: Box::new(EchoHandler),
    });

    let cmd = registry.get("echo");
    assert!(cmd.is_some(), "Command 'echo' should be found after registration");
    assert_eq!(cmd.unwrap().name, "echo");

    assert!(
        registry.get("nonexistent").is_none(),
        "Unknown command should return None"
    );
}

// ---------------------------------------------------------------------------
// test_search_commands
// ---------------------------------------------------------------------------
#[test]
fn test_search_commands() {
    let registry = build_default_commands();

    // Search by name substring
    let results = registry.search("plan");
    let names: Vec<&str> = results.iter().map(|c| c.name.as_str()).collect();
    assert!(
        names.contains(&"plan"),
        "search('plan') should return the 'plan' command"
    );
    assert!(
        names.contains(&"exit-plan"),
        "search('plan') should return the 'exit-plan' command"
    );

    // Search by description keyword (case-insensitive)
    let results = registry.search("token");
    let names: Vec<&str> = results.iter().map(|c| c.name.as_str()).collect();
    assert!(
        names.contains(&"cost"),
        "search('token') should match the 'cost' command via its description"
    );

    // Search with no results
    let results = registry.search("xyzzy_no_match_12345");
    assert!(results.is_empty(), "Nonsense query should return no results");
}

// ---------------------------------------------------------------------------
// test_help_command_output
// ---------------------------------------------------------------------------
#[test]
fn test_help_command_output() {
    let registry = build_default_commands();
    let cmd = registry.get("help").expect("'help' command must be registered");
    let ctx = make_ctx();

    let result = cmd.handler.execute("", &ctx).expect("help should not fail");
    match result {
        CommandResult::Action(text) => {
            assert!(text.contains("/help"), "help output should list /help");
            assert!(text.contains("/commit"), "help output should list /commit");
            assert!(text.contains("/status"), "help output should list /status");
        }
        _ => panic!("Expected CommandResult::Action from help command"),
    }
}

// ---------------------------------------------------------------------------
// test_commit_command_returns_prompt
// ---------------------------------------------------------------------------
#[test]
fn test_commit_command_returns_prompt() {
    let registry = build_default_commands();
    let cmd = registry.get("commit").expect("'commit' command must be registered");
    let ctx = make_ctx();

    let result = cmd.handler.execute("", &ctx).expect("commit should not fail");
    match result {
        CommandResult::Message(text) => {
            assert!(
                text.contains("git diff --cached") || text.contains("staged"),
                "commit prompt should reference staged changes or git diff --cached"
            );
        }
        _ => panic!("Expected CommandResult::Message (Prompt type) from commit command"),
    }
}

// ---------------------------------------------------------------------------
// test_clear_command
// ---------------------------------------------------------------------------
#[test]
fn test_clear_command() {
    let registry = build_default_commands();
    let cmd = registry.get("clear").expect("'clear' command must be registered");
    let ctx = make_ctx();

    let result = cmd.handler.execute("", &ctx).expect("clear should not fail");
    match result {
        CommandResult::Action(text) => {
            assert!(
                !text.is_empty(),
                "clear command should return a non-empty confirmation message"
            );
        }
        _ => panic!("Expected CommandResult::Action from clear command"),
    }
}

// ---------------------------------------------------------------------------
// test_all_builtin_commands_registered
// ---------------------------------------------------------------------------
#[test]
fn test_all_builtin_commands_registered() {
    let registry = build_default_commands();
    let all = registry.all();
    let count = all.len();

    // Verify we have a reasonable number of commands
    assert!(
        count >= 47,
        "Expected at least 47 built-in commands, found {}",
        count
    );

    // Spot-check a selection of required commands
    let required = [
        "help", "status", "clear", "compact", "model", "config", "cost",
        "permissions", "verbose", "plan", "exit-plan", "commit", "review",
        "branch", "pr", "bug", "test", "refactor", "explain", "docs",
        "memory", "tasks", "resume", "fork", "context", "theme", "fast",
        "brief", "effort",
        // Batch 1
        "doctor", "diff", "export", "mcp", "plugin", "skills", "agents",
        "rewind", "files", "init", "stats", "env", "hooks", "session",
        "copy", "pr-comments", "proactive", "ultrareview",
        // Batch 2 commands will be added when rate limit resets
    ];
    for name in &required {
        assert!(
            registry.get(name).is_some(),
            "Built-in command '{}' must be registered",
            name
        );
    }
}

// ---------------------------------------------------------------------------
// test_status_with_shared_state shows real data
// ---------------------------------------------------------------------------
#[test]
fn test_status_with_shared_state() {
    let (ctx, _shared) = make_ctx_with_shared();
    let registry = build_default_commands();
    let cmd = registry.get("status").unwrap();
    let result = cmd.handler.execute("", &ctx).unwrap();
    match result {
        CommandResult::Action(text) => {
            assert!(text.contains("claude-opus-4"), "model: {}", text);
            assert!(text.contains("Messages: 12"), "messages: {}", text);
            assert!(text.contains("Total tokens: 50000"), "tokens: {}", text);
            assert!(text.contains("abc-1234"), "session id: {}", text);
            assert!(text.contains("API requests: 5"), "requests: {}", text);
        }
        _ => panic!("Expected Action from status"),
    }
}

// ---------------------------------------------------------------------------
// test_cost_with_shared_state shows real cost summary
// ---------------------------------------------------------------------------
#[test]
fn test_cost_with_shared_state() {
    let (ctx, _shared) = make_ctx_with_shared();
    let registry = build_default_commands();
    let cmd = registry.get("cost").unwrap();
    let result = cmd.handler.execute("", &ctx).unwrap();
    match result {
        CommandResult::Action(text) => {
            assert!(text.contains("$0.3750"), "cost: {}", text);
            assert!(text.contains("30000"), "input tokens: {}", text);
        }
        _ => panic!("Expected Action from cost"),
    }
}

// ---------------------------------------------------------------------------
// test_context_with_shared_state shows utilization
// ---------------------------------------------------------------------------
#[test]
fn test_context_with_shared_state() {
    let (ctx, _shared) = make_ctx_with_shared();
    let registry = build_default_commands();
    let cmd = registry.get("context").unwrap();
    let result = cmd.handler.execute("", &ctx).unwrap();
    match result {
        CommandResult::Action(text) => {
            assert!(text.contains("50000"), "used tokens: {}", text);
            assert!(text.contains("200000"), "window size: {}", text);
            assert!(text.contains("25%"), "utilization: {}", text);
        }
        _ => panic!("Expected Action from context"),
    }
}

// ---------------------------------------------------------------------------
// test_model_switch_updates_shared_state
// ---------------------------------------------------------------------------
#[test]
fn test_model_switch_updates_shared_state() {
    let (ctx, shared) = make_ctx_with_shared();
    let registry = build_default_commands();
    let cmd = registry.get("model").unwrap();

    // Switch
    let result = cmd.handler.execute("claude-haiku-3", &ctx).unwrap();
    match result {
        CommandResult::Action(text) => {
            assert!(text.contains("claude-haiku-3"), "switch: {}", text);
        }
        _ => panic!("Expected Action"),
    }
    assert_eq!(shared.lock().unwrap().model, "claude-haiku-3");
}

// ---------------------------------------------------------------------------
// test_clear_resets_shared_state
// ---------------------------------------------------------------------------
#[test]
fn test_clear_resets_shared_state() {
    let (ctx, shared) = make_ctx_with_shared();
    let registry = build_default_commands();
    let cmd = registry.get("clear").unwrap();

    cmd.handler.execute("", &ctx).unwrap();
    let state = shared.lock().unwrap();
    assert!(state.clear_requested);
    assert_eq!(state.message_count, 0);
    assert_eq!(state.total_tokens, 0);
    assert_eq!(state.request_count, 0);
}

// ---------------------------------------------------------------------------
// test_toggle_commands_actually_toggle
// ---------------------------------------------------------------------------
#[test]
fn test_toggle_commands_actually_toggle() {
    let (ctx, shared) = make_ctx_with_shared();
    let registry = build_default_commands();

    // Fast
    let cmd = registry.get("fast").unwrap();
    cmd.handler.execute("", &ctx).unwrap();
    assert!(shared.lock().unwrap().fast_mode);
    cmd.handler.execute("", &ctx).unwrap();
    assert!(!shared.lock().unwrap().fast_mode);

    // Brief
    let cmd = registry.get("brief").unwrap();
    cmd.handler.execute("", &ctx).unwrap();
    assert!(shared.lock().unwrap().brief_mode);

    // Verbose
    let cmd = registry.get("verbose").unwrap();
    cmd.handler.execute("", &ctx).unwrap();
    assert!(shared.lock().unwrap().verbose_mode);

    // Theme
    let cmd = registry.get("theme").unwrap();
    cmd.handler.execute("", &ctx).unwrap();
    assert!(!shared.lock().unwrap().dark_theme); // was true, now false

    // Effort
    let cmd = registry.get("effort").unwrap();
    let result = cmd.handler.execute("high", &ctx).unwrap();
    match result {
        CommandResult::Action(text) => assert!(text.contains("high")),
        _ => panic!("expected Action"),
    }
    assert_eq!(shared.lock().unwrap().effort_level, "high");
}

// ---------------------------------------------------------------------------
// test_fork_sets_flag
// ---------------------------------------------------------------------------
#[test]
fn test_fork_sets_flag() {
    let (ctx, shared) = make_ctx_with_shared();
    let registry = build_default_commands();
    let cmd = registry.get("fork").unwrap();
    let result = cmd.handler.execute("", &ctx).unwrap();
    match result {
        CommandResult::Action(text) => {
            assert!(text.contains("forked"), "fork msg: {}", text);
            assert!(text.contains("New session ID"), "session id: {}", text);
        }
        _ => panic!("expected Action"),
    }
    assert!(shared.lock().unwrap().fork_requested);
}

// ---------------------------------------------------------------------------
// test_permissions_shows_mode_description
// ---------------------------------------------------------------------------
#[test]
fn test_permissions_shows_mode_description() {
    let (ctx, _shared) = make_ctx_with_shared();
    let registry = build_default_commands();
    let cmd = registry.get("permissions").unwrap();
    let result = cmd.handler.execute("", &ctx).unwrap();
    match result {
        CommandResult::Action(text) => {
            assert!(text.contains("default"), "mode: {}", text);
            assert!(text.contains("require approval") || text.contains("approval"), "description: {}", text);
        }
        _ => panic!("expected Action"),
    }
}

// ---------------------------------------------------------------------------
// test_effort_rejects_invalid_level
// ---------------------------------------------------------------------------
#[test]
fn test_effort_rejects_invalid_level() {
    let (ctx, _shared) = make_ctx_with_shared();
    let registry = build_default_commands();
    let cmd = registry.get("effort").unwrap();
    let result = cmd.handler.execute("ultra", &ctx).unwrap();
    match result {
        CommandResult::Error(text) => {
            assert!(text.contains("Invalid"), "error msg: {}", text);
        }
        _ => panic!("expected Error for invalid effort level"),
    }
}

// ---------------------------------------------------------------------------
// test_memory_handler_runs_without_panic
// ---------------------------------------------------------------------------
#[test]
fn test_memory_handler_runs_without_panic() {
    let ctx = make_ctx();
    let registry = build_default_commands();
    let cmd = registry.get("memory").unwrap();
    let result = cmd.handler.execute("", &ctx).unwrap();
    match result {
        CommandResult::Action(text) => {
            assert!(text.contains("Memory files") || text.contains("memory"), "header: {}", text);
        }
        _ => panic!("expected Action"),
    }
}

// ---------------------------------------------------------------------------
// test_resume_runs_without_panic
// ---------------------------------------------------------------------------
#[test]
fn test_resume_runs_without_panic() {
    let ctx = make_ctx();
    let registry = build_default_commands();
    let cmd = registry.get("resume").unwrap();
    let result = cmd.handler.execute("", &ctx).unwrap();
    // Either lists sessions or says none found
    match result {
        CommandResult::Action(text) => {
            assert!(
                text.contains("sessions") || text.contains("No previous"),
                "resume: {}",
                text
            );
        }
        CommandResult::Error(_) => {} // acceptable if dir doesn't exist
        _ => panic!("expected Action or Error"),
    }
}

// ---------------------------------------------------------------------------
// test_fallback_without_shared_state
// ---------------------------------------------------------------------------
#[test]
fn test_fallback_without_shared_state() {
    let ctx = make_ctx();
    let registry = build_default_commands();

    for name in &["status", "cost", "context"] {
        let cmd = registry.get(name).unwrap();
        let result = cmd.handler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(
                    text.contains("no live session") || text.contains("No usage"),
                    "/{} should show fallback without shared state, got: {}",
                    name,
                    text
                );
            }
            _ => panic!("expected Action for /{}", name),
        }
    }
}
