use std::path::Path;

use crate::plugins::types::{
    Plugin, PluginCommand, PluginLoadResult, PluginManifest, PluginSettings, PluginSource,
};

/// Scan a directory for plugin manifests and load them.
///
/// Each immediate subdirectory of `dir` that contains a `plugin.json` file
/// is treated as a plugin. The manifest is parsed, its skills/commands
/// sub-directories are scanned, and the result is assembled into a
/// `Plugin` struct.
///
/// This mirrors the TypeScript plugin loader that walks marketplace
/// install directories.
pub fn load_plugins_from_dir(dir: &Path, settings: &PluginSettings) -> PluginLoadResult {
    let mut result = PluginLoadResult::default();

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return result,
    };

    for entry in entries.flatten() {
        let plugin_dir = entry.path();
        if !plugin_dir.is_dir() {
            continue;
        }

        let manifest_path = plugin_dir.join("plugin.json");
        let manifest_text = match std::fs::read_to_string(&manifest_path) {
            Ok(t) => t,
            Err(_) => continue, // not a plugin directory
        };

        let manifest: PluginManifest = match serde_json::from_str(&manifest_text) {
            Ok(m) => m,
            Err(e) => {
                result.errors.push(format!(
                    "Failed to parse {}: {}",
                    manifest_path.display(),
                    e
                ));
                continue;
            }
        };

        // Determine plugin id and enabled state
        let plugin_id = format!("{}@marketplace", manifest.name);
        let enabled = settings
            .enabled_plugins
            .get(&plugin_id)
            .copied()
            .unwrap_or(true);

        // Discover commands/skills within the plugin directory
        let commands = load_plugin_commands(&plugin_dir);

        let plugin = Plugin {
            id: plugin_id,
            name: manifest.name.clone(),
            version: manifest.version,
            description: manifest.description,
            commands,
            enabled,
            source: PluginSource::Marketplace {
                name: "marketplace".to_string(),
            },
        };

        if enabled {
            result.enabled.push(plugin);
        } else {
            result.disabled.push(plugin);
        }
    }

    result
}

/// Load commands from the `skills/` subdirectory of a plugin.
///
/// Each subdirectory containing a `SKILL.md` is converted into a
/// `PluginCommand`.
fn load_plugin_commands(plugin_dir: &Path) -> Vec<PluginCommand> {
    let mut commands = Vec::new();

    // Check skills/ subdirectory
    let skills_dir = plugin_dir.join("skills");
    if let Ok(entries) = std::fs::read_dir(&skills_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let skill_file = path.join("SKILL.md");
            let content = match std::fs::read_to_string(&skill_file) {
                Ok(c) => c,
                Err(_) => continue,
            };

            let parsed = crate::plugins::skill::parse_skill_file(&content);
            let dir_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");

            let name = parsed
                .frontmatter
                .name
                .unwrap_or_else(|| dir_name.to_string());
            let description = parsed
                .frontmatter
                .description
                .unwrap_or_else(|| format!("Command: {}", name));

            commands.push(PluginCommand {
                name,
                description,
                prompt_template: parsed.content,
            });
        }
    }

    // Also check commands/ subdirectory (legacy format)
    let commands_dir = plugin_dir.join("commands");
    if let Ok(entries) = std::fs::read_dir(&commands_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let is_md = path.extension().map(|e| e == "md").unwrap_or(false);

            if !is_md {
                continue;
            }

            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            let parsed = crate::plugins::skill::parse_skill_file(&content);
            let file_stem = path
                .file_stem()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");

            let name = parsed
                .frontmatter
                .name
                .unwrap_or_else(|| file_stem.to_string());
            let description = parsed
                .frontmatter
                .description
                .unwrap_or_else(|| format!("Command: {}", name));

            commands.push(PluginCommand {
                name,
                description,
                prompt_template: parsed.content,
            });
        }
    }

    commands
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn create_test_plugin(dir: &Path, name: &str, version: &str) {
        let plugin_dir = dir.join(name);
        fs::create_dir_all(&plugin_dir).unwrap();

        let manifest = serde_json::json!({
            "name": name,
            "description": format!("Test plugin {}", name),
            "version": version,
        });
        fs::write(
            plugin_dir.join("plugin.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        // Add a skill
        let skill_dir = plugin_dir.join("skills").join("greet");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: greet\ndescription: Say hello\n---\nHello, world!",
        )
        .unwrap();
    }

    #[test]
    fn load_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let result = load_plugins_from_dir(tmp.path(), &PluginSettings::default());
        assert!(result.enabled.is_empty());
        assert!(result.disabled.is_empty());
        assert!(result.errors.is_empty());
    }

    #[test]
    fn load_nonexistent_dir() {
        let result = load_plugins_from_dir(
            Path::new("/tmp/claude_test_nonexistent_plugin_dir_xyz"),
            &PluginSettings::default(),
        );
        assert!(result.enabled.is_empty());
    }

    #[test]
    fn load_plugin_with_skill() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_plugin(tmp.path(), "test-plugin", "1.0.0");

        let result = load_plugins_from_dir(tmp.path(), &PluginSettings::default());
        assert_eq!(result.enabled.len(), 1);
        assert_eq!(result.disabled.len(), 0);

        let plugin = &result.enabled[0];
        assert_eq!(plugin.name, "test-plugin");
        assert_eq!(plugin.version, "1.0.0");
        assert!(plugin.enabled);
        assert_eq!(plugin.commands.len(), 1);
        assert_eq!(plugin.commands[0].name, "greet");
        assert_eq!(plugin.commands[0].description, "Say hello");
        assert_eq!(plugin.commands[0].prompt_template, "Hello, world!");
    }

    #[test]
    fn load_plugin_disabled_by_settings() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_plugin(tmp.path(), "disabled-plugin", "0.1.0");

        let mut settings = PluginSettings::default();
        settings
            .enabled_plugins
            .insert("disabled-plugin@marketplace".to_string(), false);

        let result = load_plugins_from_dir(tmp.path(), &settings);
        assert_eq!(result.enabled.len(), 0);
        assert_eq!(result.disabled.len(), 1);
        assert_eq!(result.disabled[0].name, "disabled-plugin");
        assert!(!result.disabled[0].enabled);
    }

    #[test]
    fn load_plugin_invalid_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let plugin_dir = tmp.path().join("bad-plugin");
        fs::create_dir_all(&plugin_dir).unwrap();
        fs::write(plugin_dir.join("plugin.json"), "not valid json!!!").unwrap();

        let result = load_plugins_from_dir(tmp.path(), &PluginSettings::default());
        assert!(result.enabled.is_empty());
        assert_eq!(result.errors.len(), 1);
        assert!(result.errors[0].contains("bad-plugin"));
    }
}
