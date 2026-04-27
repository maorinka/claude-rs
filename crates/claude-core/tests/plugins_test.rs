use claude_core::plugins::loader::load_plugins_from_dir;
use claude_core::plugins::skill::{discover_skills, match_skill, parse_skill_file};
use claude_core::plugins::types::*;
use std::fs;

// ---------------------------------------------------------------------------
// Skill file parsing
// ---------------------------------------------------------------------------

#[test]
fn test_parse_skill_file_no_frontmatter() {
    let content = "# My Skill\n\nJust some markdown.";
    let parsed = parse_skill_file(content);
    assert!(parsed.frontmatter.name.is_none());
    assert!(parsed.frontmatter.description.is_none());
    assert_eq!(parsed.content, content);
}

#[test]
fn test_parse_skill_file_full_frontmatter() {
    let content = r#"---
name: deploy
description: Deploy the application
argument-hint: "<environment>"
when_to_use: When the user asks to deploy
allowed-tools:
  - Bash
  - Read
user-invocable: true
disable-model-invocation: true
---
Run the deploy script for the given environment.
"#;
    let parsed = parse_skill_file(content);
    assert_eq!(parsed.frontmatter.name.as_deref(), Some("deploy"));
    assert_eq!(
        parsed.frontmatter.description.as_deref(),
        Some("Deploy the application")
    );
    assert_eq!(
        parsed.frontmatter.argument_hint.as_deref(),
        Some("<environment>")
    );
    assert_eq!(
        parsed.frontmatter.when_to_use.as_deref(),
        Some("When the user asks to deploy")
    );
    assert_eq!(parsed.frontmatter.allowed_tools, vec!["Bash", "Read"]);
    assert_eq!(parsed.frontmatter.user_invocable, Some(true));
    assert_eq!(parsed.frontmatter.disable_model_invocation, Some(true));
    assert!(parsed.content.contains("Run the deploy script"));
    // Frontmatter should NOT appear in the content body
    assert!(!parsed.content.contains("name: deploy"));
}

#[test]
fn test_parse_skill_file_empty_frontmatter() {
    let content = "---\n---\nBody only.";
    let parsed = parse_skill_file(content);
    assert!(parsed.frontmatter.name.is_none());
    assert_eq!(parsed.content, "Body only.");
}

#[test]
fn test_parse_skill_file_allowed_tools_csv() {
    let content = "---\nallowed-tools: Bash, Read, Write\n---\ncontent";
    let parsed = parse_skill_file(content);
    assert_eq!(
        parsed.frontmatter.allowed_tools,
        vec!["Bash", "Read", "Write"]
    );
}

#[test]
fn test_parse_skill_file_boolean_variants() {
    // Test various boolean representations
    for (val, expected) in [
        ("true", true),
        ("yes", true),
        ("on", true),
        ("false", false),
        ("no", false),
        ("off", false),
    ] {
        let content = format!("---\nuser-invocable: {}\n---\nbody", val);
        let parsed = parse_skill_file(&content);
        assert_eq!(
            parsed.frontmatter.user_invocable,
            Some(expected),
            "Failed for value: {}",
            val
        );
    }
}

// ---------------------------------------------------------------------------
// Skill discovery (filesystem)
// ---------------------------------------------------------------------------

#[test]
fn test_discover_skills_in_temp_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let skills_dir = tmp.path().join(".claude").join("skills");

    // Create two skills
    let skill1_dir = skills_dir.join("greet");
    fs::create_dir_all(&skill1_dir).unwrap();
    fs::write(
        skill1_dir.join("SKILL.md"),
        "---\nname: greet\ndescription: Greet the user\n---\nHello {{name}}!",
    )
    .unwrap();

    let skill2_dir = skills_dir.join("farewell");
    fs::create_dir_all(&skill2_dir).unwrap();
    fs::write(
        skill2_dir.join("SKILL.md"),
        "---\ndescription: Say goodbye\n---\nGoodbye!",
    )
    .unwrap();

    // Create a non-skill file (should be ignored)
    fs::write(skills_dir.join("random.txt"), "not a skill").unwrap();

    let skills = discover_skills(tmp.path());

    // Should find skills from the project directory
    // (user-level skills depend on actual ~/.claude/skills which we can't control in test)
    let project_skills: Vec<_> = skills
        .iter()
        .filter(|s| matches!(&s.source, SkillSource::Directory(_)))
        .collect();

    assert!(
        project_skills.len() >= 2,
        "Expected at least 2 project skills, got {}",
        project_skills.len()
    );

    let greet = project_skills.iter().find(|s| s.name == "greet");
    assert!(greet.is_some(), "Should find 'greet' skill");
    let greet = greet.unwrap();
    assert_eq!(greet.description, "Greet the user");
    assert!(greet.content.contains("Hello {{name}}!"));

    // The farewell skill should use the directory name since no name in frontmatter
    let farewell = project_skills.iter().find(|s| s.name == "farewell");
    assert!(farewell.is_some(), "Should find 'farewell' skill");
    assert_eq!(farewell.unwrap().description, "Say goodbye");
}

#[test]
fn test_discover_skills_empty_dir() {
    let tmp = tempfile::tempdir().unwrap();
    // No .claude/skills/ directory at all
    let skills = discover_skills(tmp.path());
    // May include user-level skills, but should not panic
    assert!(skills.iter().all(|s| !s.name.is_empty()));
}

#[test]
fn test_discover_skills_skips_non_directories() {
    let tmp = tempfile::tempdir().unwrap();
    let skills_dir = tmp.path().join(".claude").join("skills");
    fs::create_dir_all(&skills_dir).unwrap();

    // A plain file in skills/ (not a subdirectory) should be ignored
    fs::write(
        skills_dir.join("not-a-skill.md"),
        "---\nname: bad\n---\nbody",
    )
    .unwrap();

    let skills = discover_skills(tmp.path());
    let project_skills: Vec<_> = skills
        .iter()
        .filter(|s| matches!(&s.source, SkillSource::Directory(_)))
        .collect();
    // Should not pick up the plain file
    assert!(
        !project_skills
            .iter()
            .any(|s| s.name == "bad" || s.name == "not-a-skill"),
        "Plain files in skills/ should not be loaded"
    );
}

// ---------------------------------------------------------------------------
// Skill matching
// ---------------------------------------------------------------------------

fn test_skill(name: &str, when_to_use: Option<&str>) -> Skill {
    Skill {
        name: name.to_string(),
        description: format!("Test skill: {}", name),
        content: String::new(),
        source: SkillSource::Builtin,
        argument_hint: None,
        when_to_use: when_to_use.map(String::from),
        allowed_tools: vec![],
        user_invocable: true,
        disable_model_invocation: false,
        is_plugin_command: false,
    }
}

#[test]
fn test_match_skill_slash_command() {
    let skill = test_skill("deploy", None);
    assert_eq!(match_skill("/deploy", &skill), Some(""));
    assert_eq!(match_skill("/deploy staging", &skill), Some("staging"));
    assert_eq!(
        match_skill("/deploy --env prod", &skill),
        Some("--env prod")
    );
}

#[test]
fn test_match_skill_no_match() {
    let skill = test_skill("deploy", None);
    assert_eq!(match_skill("/other", &skill), None);
    assert_eq!(match_skill("deploy", &skill), None); // no slash prefix
    assert_eq!(match_skill("", &skill), None);
}

#[test]
fn test_match_skill_partial_name_no_match() {
    let skill = test_skill("deploy", None);
    // /deployer should not match /deploy
    assert_eq!(match_skill("/deployer", &skill), None);
}

#[test]
fn test_match_skill_fuzzy_with_when_to_use() {
    let skill = test_skill("deploy", Some("When deploying the application"));
    assert_eq!(match_skill("please deploy the app", &skill), Some(""));
    // Substring of a word should NOT match
    assert_eq!(match_skill("undeployable", &skill), None);
}

#[test]
fn test_match_skill_no_fuzzy_without_when_to_use() {
    let skill = test_skill("deploy", None);
    // Without when_to_use, only slash-command should match
    assert_eq!(match_skill("please deploy the app", &skill), None);
}

// ---------------------------------------------------------------------------
// Plugin types serialization
// ---------------------------------------------------------------------------

#[test]
fn test_plugin_manifest_deserialize() {
    let json = r#"{"name": "test", "description": "A test plugin", "version": "1.2.3"}"#;
    let manifest: PluginManifest = serde_json::from_str(json).unwrap();
    assert_eq!(manifest.name, "test");
    assert_eq!(manifest.description, "A test plugin");
    assert_eq!(manifest.version, "1.2.3");
}

#[test]
fn test_plugin_manifest_default_version() {
    let json = r#"{"name": "test"}"#;
    let manifest: PluginManifest = serde_json::from_str(json).unwrap();
    assert_eq!(manifest.version, "0.0.0");
    assert_eq!(manifest.description, "");
}

#[test]
fn test_plugin_roundtrip() {
    let plugin = Plugin {
        id: "my-plugin@builtin".to_string(),
        name: "my-plugin".to_string(),
        version: "1.0.0".to_string(),
        description: "A test plugin".to_string(),
        commands: vec![PluginCommand {
            name: "hello".to_string(),
            description: "Say hello".to_string(),
            prompt_template: "Hello!".to_string(),
        }],
        enabled: true,
        source: PluginSource::Builtin,
    };

    let json = serde_json::to_string(&plugin).unwrap();
    let deserialized: Plugin = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.id, plugin.id);
    assert_eq!(deserialized.name, plugin.name);
    assert_eq!(deserialized.commands.len(), 1);
    assert_eq!(deserialized.commands[0].name, "hello");
    assert!(deserialized.enabled);
}

#[test]
fn test_skill_source_variants() {
    // Builtin
    let json = serde_json::to_string(&SkillSource::Builtin).unwrap();
    let back: SkillSource = serde_json::from_str(&json).unwrap();
    assert_eq!(back, SkillSource::Builtin);

    // Directory
    let dir = SkillSource::Directory("/home/user/.claude/skills".into());
    let json = serde_json::to_string(&dir).unwrap();
    let back: SkillSource = serde_json::from_str(&json).unwrap();
    assert_eq!(back, dir);

    // Plugin
    let plugin = SkillSource::Plugin("my-plugin@marketplace".to_string());
    let json = serde_json::to_string(&plugin).unwrap();
    let back: SkillSource = serde_json::from_str(&json).unwrap();
    assert_eq!(back, plugin);
}

#[test]
fn test_plugin_settings_deserialize() {
    let json = r#"{"enabled_plugins": {"foo@builtin": true, "bar@marketplace": false}}"#;
    let settings: PluginSettings = serde_json::from_str(json).unwrap();
    assert_eq!(settings.enabled_plugins.get("foo@builtin"), Some(&true));
    assert_eq!(
        settings.enabled_plugins.get("bar@marketplace"),
        Some(&false)
    );
}

// ---------------------------------------------------------------------------
// Plugin loading (filesystem)
// ---------------------------------------------------------------------------

#[test]
fn test_load_plugins_from_dir_with_commands_subdir() {
    let tmp = tempfile::tempdir().unwrap();
    let plugin_dir = tmp.path().join("my-plugin");
    fs::create_dir_all(&plugin_dir).unwrap();

    // Write manifest
    fs::write(
        plugin_dir.join("plugin.json"),
        r#"{"name": "my-plugin", "description": "Test", "version": "2.0.0"}"#,
    )
    .unwrap();

    // Add a command via commands/ directory (legacy format)
    let cmds_dir = plugin_dir.join("commands");
    fs::create_dir_all(&cmds_dir).unwrap();
    fs::write(
        cmds_dir.join("check.md"),
        "---\nname: check\ndescription: Run checks\n---\nRun all checks.",
    )
    .unwrap();

    let result = load_plugins_from_dir(tmp.path(), &PluginSettings::default());
    assert_eq!(result.enabled.len(), 1);

    let plugin = &result.enabled[0];
    assert_eq!(plugin.name, "my-plugin");
    assert_eq!(plugin.commands.len(), 1);
    assert_eq!(plugin.commands[0].name, "check");
    assert_eq!(plugin.commands[0].description, "Run checks");
}

#[test]
fn test_load_plugins_multiple() {
    let tmp = tempfile::tempdir().unwrap();

    for (name, ver) in [("alpha", "1.0.0"), ("beta", "2.0.0")] {
        let pd = tmp.path().join(name);
        fs::create_dir_all(&pd).unwrap();
        fs::write(
            pd.join("plugin.json"),
            format!(
                r#"{{"name": "{}", "description": "Plugin {}", "version": "{}"}}"#,
                name, name, ver
            ),
        )
        .unwrap();
    }

    let result = load_plugins_from_dir(tmp.path(), &PluginSettings::default());
    assert_eq!(result.enabled.len(), 2);

    let names: Vec<_> = result.enabled.iter().map(|p| p.name.as_str()).collect();
    assert!(names.contains(&"alpha"));
    assert!(names.contains(&"beta"));
}
