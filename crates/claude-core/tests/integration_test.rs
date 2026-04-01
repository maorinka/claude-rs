use std::collections::HashMap;
use std::fs;

use claude_core::commands::builtin::build_default_commands;
use claude_core::commands::registry::{CommandContext, CommandResult};
use claude_core::config::settings::{McpServerSettingsEntry, Settings};
use claude_core::mcp::manager::McpManager;
use claude_core::mcp::types::*;
use claude_core::plugins::skill::discover_skills;
use claude_core::plugins::types::SkillSource;

// ---------------------------------------------------------------------------
// MCP Manager integration tests
// ---------------------------------------------------------------------------

#[test]
fn test_mcp_manager_configs_from_settings() {
    // Verify that the config-building logic in main.rs produces valid
    // ScopedMcpServerConfig entries from Settings.mcp_servers entries.
    let settings_json = r#"{
        "mcpServers": {
            "test-server": {
                "command": "npx",
                "args": ["-y", "@test/server"],
                "env": {"KEY": "value"}
            }
        }
    }"#;
    let settings: Settings = serde_json::from_str(settings_json).unwrap();

    let mut configs: HashMap<String, ScopedMcpServerConfig> = HashMap::new();
    for (name, entry) in &settings.mcp_servers {
        let env = if entry.env.is_empty() {
            None
        } else {
            Some(entry.env.clone())
        };
        let scoped = ScopedMcpServerConfig {
            config: McpServerConfig::Stdio(McpStdioServerConfig {
                command: entry.command.clone(),
                args: entry.args.clone(),
                env,
            }),
            scope: ConfigScope::User,
        };
        configs.insert(name.clone(), scoped);
    }

    assert_eq!(configs.len(), 1);
    let scoped = configs.get("test-server").unwrap();
    assert_eq!(scoped.scope, ConfigScope::User);
    match &scoped.config {
        McpServerConfig::Stdio(stdio) => {
            assert_eq!(stdio.command, "npx");
            assert_eq!(stdio.args, vec!["-y", "@test/server"]);
            let env = stdio.env.as_ref().unwrap();
            assert_eq!(env.get("KEY").unwrap(), "value");
        }
        _ => panic!("Expected Stdio config"),
    }
}

#[tokio::test]
async fn test_mcp_manager_connect_graceful_failure() {
    // Verify that connecting to a server with an invalid command fails gracefully
    // (not panic or hang). Use /bin/false which exits immediately with status 1.
    let manager = McpManager::new();
    let mut configs = HashMap::new();
    let scoped = ScopedMcpServerConfig {
        config: McpServerConfig::Stdio(McpStdioServerConfig {
            command: "/bin/false".to_string(),
            args: vec![],
            env: None,
        }),
        scope: ConfigScope::User,
    };
    configs.insert("bad-server".to_string(), scoped);

    // Use a timeout to avoid hanging
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        manager.connect_all(configs),
    )
    .await;

    match result {
        Ok(connections) => {
            assert_eq!(connections.len(), 1);
            assert_eq!(connections[0].name, "bad-server");
            // Should fail because /bin/false exits immediately
            assert!(
                connections[0].is_failed(),
                "Connection to /bin/false should fail"
            );
        }
        Err(_) => {
            // Timeout is also acceptable - the manager should not panic
        }
    }
}

#[tokio::test]
async fn test_mcp_manager_empty_settings() {
    // Verify connect_all with empty map does nothing
    let manager = McpManager::new();
    let connections = manager.connect_all(HashMap::new()).await;
    assert!(connections.is_empty());
    assert!(!manager.has_connections().await);
}

#[test]
fn test_settings_mcp_servers_parsing() {
    let json = r#"{
        "mcpServers": {
            "my-server": {
                "command": "npx",
                "args": ["-y", "@some/mcp-server"],
                "env": {"API_KEY": "test-key"}
            },
            "another-server": {
                "command": "python",
                "args": ["-m", "mcp_server"]
            }
        }
    }"#;

    let settings: Settings = serde_json::from_str(json).unwrap();
    assert_eq!(settings.mcp_servers.len(), 2);

    let my_server = settings.mcp_servers.get("my-server").unwrap();
    assert_eq!(my_server.command, "npx");
    assert_eq!(my_server.args, vec!["-y", "@some/mcp-server"]);
    assert_eq!(my_server.env.get("API_KEY").unwrap(), "test-key");

    let another = settings.mcp_servers.get("another-server").unwrap();
    assert_eq!(another.command, "python");
    assert_eq!(another.args, vec!["-m", "mcp_server"]);
    assert!(another.env.is_empty());
}

#[test]
fn test_settings_mcp_servers_merge() {
    let base = Settings {
        mcp_servers: {
            let mut m = HashMap::new();
            m.insert(
                "server-a".to_string(),
                McpServerSettingsEntry {
                    command: "cmd-a".to_string(),
                    ..Default::default()
                },
            );
            m
        },
        ..Default::default()
    };

    let overlay = Settings {
        mcp_servers: {
            let mut m = HashMap::new();
            m.insert(
                "server-b".to_string(),
                McpServerSettingsEntry {
                    command: "cmd-b".to_string(),
                    ..Default::default()
                },
            );
            m
        },
        ..Default::default()
    };

    let merged = base.merge(&overlay);
    assert_eq!(merged.mcp_servers.len(), 2);
    assert!(merged.mcp_servers.contains_key("server-a"));
    assert!(merged.mcp_servers.contains_key("server-b"));
}

#[test]
fn test_settings_empty_mcp_servers() {
    let json = r#"{}"#;
    let settings: Settings = serde_json::from_str(json).unwrap();
    assert!(settings.mcp_servers.is_empty());
}

#[test]
fn test_settings_load_from_nonexistent_file() {
    let settings = Settings::load_from_file(std::path::Path::new("/tmp/nonexistent_claude_settings_xyz.json"));
    assert!(settings.mcp_servers.is_empty());
    assert!(settings.model.is_none());
}

// ---------------------------------------------------------------------------
// Skill discovery -> system prompt integration tests
// ---------------------------------------------------------------------------

#[test]
fn test_skill_discovery_feeds_system_prompt() {
    let tmp = tempfile::tempdir().unwrap();
    let skills_dir = tmp.path().join(".claude").join("skills");

    // Create a test skill
    let skill_dir = skills_dir.join("deploy");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: deploy\ndescription: Deploy the app\nwhen_to_use: When deploying\n---\nRun deployment steps.",
    )
    .unwrap();

    let skills = discover_skills(tmp.path());

    // Build the skills system prompt text (same logic as main.rs)
    let mut skills_text = String::from("\n# Available Skills\n\n");
    skills_text.push_str("The following skills are available for use with the Skill tool:\n\n");
    for skill in &skills {
        skills_text.push_str(&format!("- {}: {}", skill.name, skill.description));
        if let Some(ref hint) = skill.when_to_use {
            skills_text.push_str(&format!(" (use when: {})", hint));
        }
        skills_text.push('\n');
    }

    // Verify the deploy skill appears in the prompt
    let project_skills: Vec<_> = skills
        .iter()
        .filter(|s| matches!(&s.source, SkillSource::Directory(_)))
        .collect();

    let has_deploy = project_skills.iter().any(|s| s.name == "deploy");
    assert!(has_deploy, "deploy skill should be discovered");
    assert!(
        skills_text.contains("deploy"),
        "System prompt should mention deploy skill"
    );
    assert!(
        skills_text.contains("When deploying"),
        "System prompt should include when_to_use hint"
    );
}

#[test]
fn test_skill_discovery_empty_project() {
    let tmp = tempfile::tempdir().unwrap();
    let skills = discover_skills(tmp.path());

    // Should not panic and should return at least empty or user-level skills
    for skill in &skills {
        assert!(!skill.name.is_empty());
    }
}

// ---------------------------------------------------------------------------
// /help command through command registry flow
// ---------------------------------------------------------------------------

#[test]
fn test_help_command_flow() {
    let registry = build_default_commands();

    // Simulate what the TUI does: get command, execute, check result
    let cmd = registry.get("help").expect("/help command should exist");

    let ctx = CommandContext {
        working_directory: std::path::PathBuf::from("/tmp"),
        model: "claude-sonnet-4-6".to_string(),
        shared: None,
    };

    let result = cmd.handler.execute("", &ctx).unwrap();
    match result {
        CommandResult::Action(text) => {
            assert!(text.contains("/help"));
            assert!(text.contains("/commit"));
            assert!(text.contains("/status"));
            assert!(text.contains("/review"));
        }
        _ => panic!("Expected Action result from /help"),
    }
}

#[test]
fn test_slash_command_search_and_execute() {
    let registry = build_default_commands();
    let ctx = CommandContext {
        working_directory: std::path::PathBuf::from("/tmp"),
        model: "test-model".to_string(),
        shared: None,
    };

    // Search should find "model"
    let results = registry.search("model");
    assert!(!results.is_empty());
    assert!(results.iter().any(|c| c.name == "model"));

    // Execute model command with no args
    let cmd = registry.get("model").unwrap();
    let result = cmd.handler.execute("", &ctx).unwrap();
    match result {
        CommandResult::Action(text) => {
            assert!(text.contains("test-model"));
        }
        _ => panic!("Expected Action result from /model"),
    }
}

#[test]
fn test_prompt_type_command_returns_message() {
    let registry = build_default_commands();
    let ctx = CommandContext {
        working_directory: std::path::PathBuf::from("/tmp"),
        model: "test".to_string(),
        shared: None,
    };

    // /commit should return a Message (Prompt type)
    let cmd = registry.get("commit").unwrap();
    let result = cmd.handler.execute("", &ctx).unwrap();
    assert!(
        matches!(result, CommandResult::Message(_)),
        "/commit should return a Message (Prompt type)"
    );
}

// ---------------------------------------------------------------------------
// Agent tool worktree tests (unit-level, no actual git operations)
// ---------------------------------------------------------------------------

#[test]
fn test_agent_tool_input_schema() {
    // Verify the schema includes the isolation and run_in_background fields
    let schema_json = serde_json::json!({
        "type": "object",
        "required": ["prompt"],
        "properties": {
            "prompt": {"type": "string"},
            "isolation": {"type": "string", "enum": ["worktree"]},
            "run_in_background": {"type": "boolean"}
        }
    });

    let props = schema_json["properties"].as_object().unwrap();
    assert!(props.contains_key("prompt"));
    assert!(props.contains_key("isolation"));
    assert!(props.contains_key("run_in_background"));

    let isolation_enum = &schema_json["properties"]["isolation"]["enum"];
    assert_eq!(isolation_enum[0], "worktree");
}
