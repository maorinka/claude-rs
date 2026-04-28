use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Where a skill was loaded from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SkillSource {
    /// Ships with the CLI binary.
    Builtin,
    /// Loaded from a `.claude/skills/` directory on disk.
    Directory(PathBuf),
    /// Provided by a plugin.
    Plugin(String),
}

/// Where a loaded plugin originated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginSource {
    /// Ships with the CLI binary.
    Builtin,
    /// Installed from a marketplace or git repository.
    Marketplace { name: String },
}

/// A single skill — a named prompt template with metadata.
///
/// Mirrors the TypeScript `Command` structure for skills: a skill has a name,
/// description, the markdown prompt content, and metadata parsed from its
/// YAML frontmatter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skill {
    /// Skill name used as the slash-command identifier (e.g. `"commit"`).
    pub name: String,
    /// Human-readable description shown in listings and help text.
    pub description: String,
    /// The markdown body of the skill (everything after the frontmatter).
    pub content: String,
    /// Where this skill was loaded from.
    pub source: SkillSource,
    /// Hint about the expected argument(s), shown in UI (e.g. `"<message>"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub argument_hint: Option<String>,
    /// Free-text guidance for the model on *when* to invoke this skill
    /// automatically.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when_to_use: Option<String>,
    /// Optional cwd-relative path patterns that gate when this skill becomes
    /// available. TS stores these skills and activates them after matching file
    /// operations instead of listing them at startup.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    /// Optional list of tool names this skill is allowed to use.
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// Whether the user can type `/name` to invoke the skill directly.
    #[serde(default = "default_true")]
    pub user_invocable: bool,
    /// Whether the model should be prevented from invoking this skill itself.
    #[serde(default)]
    pub disable_model_invocation: bool,
    /// True when this came from a plugin `commands/*.md` file rather than a
    /// plugin `skills/<name>/SKILL.md` directory. TS exposes both as slash
    /// commands, but only skill-directory entries are listed in
    /// `system/init.skills`.
    #[serde(default)]
    pub is_plugin_command: bool,
}

fn default_true() -> bool {
    true
}

/// A command exposed by a plugin (loaded from `commands/` or `skills/`
/// directories within the plugin tree).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginCommand {
    /// Command name (slash-command identifier).
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// The markdown prompt template for this command.
    pub prompt_template: String,
}

/// Manifest metadata embedded in a plugin's `plugin.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginManifest {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_version")]
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<PluginAuthor>,
}

fn default_version() -> String {
    "0.0.0".to_string()
}

/// Author information from a plugin manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginAuthor {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// A fully-loaded plugin, ready for use.
///
/// Corresponds to the TypeScript `LoadedPlugin` type. A plugin aggregates
/// one or more skills/commands and optional hook/MCP-server configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plugin {
    /// Unique identifier for the plugin (e.g. `"foo@builtin"` or
    /// `"bar@marketplace"`).
    pub id: String,
    /// Human-readable display name.
    pub name: String,
    /// SemVer version string.
    pub version: String,
    /// Description shown in the `/plugin` UI.
    pub description: String,
    /// Commands/skills this plugin provides.
    #[serde(default)]
    pub commands: Vec<PluginCommand>,
    /// Whether the plugin is currently active.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Where the plugin came from.
    pub source: PluginSource,
}

/// Aggregate result returned by the plugin loader.
#[derive(Debug, Clone, Default)]
pub struct PluginLoadResult {
    pub enabled: Vec<Plugin>,
    pub disabled: Vec<Plugin>,
    pub errors: Vec<String>,
}

/// Settings that influence which plugins/skills are loaded.
///
/// These mirror the per-source enable/disable toggles from the TS codebase
/// (`enabledPlugins` in user settings, `isSettingSourceEnabled`, etc.).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginSettings {
    /// Per-plugin-id enable/disable overrides.
    #[serde(default)]
    pub enabled_plugins: HashMap<String, bool>,
}
