use claude_core::commands::builtin::build_default_commands;
use claude_core::commands::registry::{
    Command, CommandContext, CommandHandler, CommandRegistry, CommandResult, CommandType,
};
use std::path::PathBuf;
use anyhow::Result;

fn make_ctx() -> CommandContext {
    CommandContext {
        working_directory: PathBuf::from("/tmp"),
        model: "claude-opus-4".to_string(),
    }
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

    // 17 Action commands + 12 Prompt commands = 29 total
    assert_eq!(
        count, 29,
        "Expected 29 built-in commands, found {}",
        count
    );

    // Spot-check a selection of required commands
    let required = [
        "help", "status", "clear", "compact", "model", "config", "cost",
        "permissions", "verbose", "plan", "exit-plan", "commit", "review",
        "branch", "pr", "bug", "test", "refactor", "explain", "docs",
        "memory", "tasks", "resume", "fork", "context", "theme", "fast",
        "brief", "effort",
    ];
    for name in &required {
        assert!(
            registry.get(name).is_some(),
            "Built-in command '{}' must be registered",
            name
        );
    }
}
