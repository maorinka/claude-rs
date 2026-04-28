//! Unified `Command` trait — slash commands, skills, plugin
//! commands, MCP prompts — all behind one interface.
//!
//! Port of TS `types/command.ts:205` (`Command = CommandBase &
//! (PromptCommand | LocalCommand | LocalJSXCommand)`) +
//! `commands.ts` usage patterns.
//!
//! **Step 3b** of the design rollout (Codex CLI gpt-5.4,
//! 2026-04-20). Codex Q6 verdict implemented verbatim:
//!
//! > "Unify command execution behind **one trait/object
//! > model**, with built-ins implemented in Rust and plugin-
//! > loaded commands adapted into the same interface. In
//! > other words, 'built-in vs plugin' is a loading concern,
//! > not a type-system concern. Define a core command trait
//! > with metadata plus an `execute` method returning a
//! > structured `CommandOutcome` enum; include a
//! > `UiNode`/`RenderSpec` variant only in the frontend-facing
//! > crate if you truly need interactive command UI. Do not
//! > port `LocalJSXCommand` literally into core; that is UI
//! > embedding disguised as command logic."
//!
//! # Scope
//!
//! - `Command` trait: minimal interface — metadata +
//!   `execute`. Metadata lives as a separate data struct so it
//!   can be snapshot cheaply (e.g. for `/help` rendering).
//! - `CommandOutcome` enum: the five shapes a command can
//!   produce. No React / JSX variant in core — UI-interactive
//!   commands produce a structured `RenderSpec` (below) that
//!   the frontend crate maps to its widget tree. Kept
//!   deliberately minimal so new outcome variants stay rare.
//! - `CommandMetadata` struct: all the purely-data fields TS
//!   `CommandBase` carries (name, description, availability,
//!   etc.), suitable for registry storage + serialisation.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use thiserror::Error;

/// Auth/provider availability gate. TS `CommandAvailability` at
/// `types/command.ts:169-173`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CommandAvailability {
    /// Claude.ai subscriber (Pro/Max/Team/Enterprise).
    ClaudeAi,
    /// Direct Console API-key user.
    Console,
}

/// Where the command was loaded from. TS `CommandBase.loadedFrom`
/// at `types/command.ts:191-197`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandLoadedFrom {
    #[serde(rename = "commands_DEPRECATED")]
    CommandsDeprecated,
    Skills,
    Plugin,
    Managed,
    Bundled,
    Mcp,
}

/// Pure-data command metadata — everything TS `CommandBase`
/// carries EXCEPT the function-typed fields (`isEnabled`,
/// `userFacingName`). Those live on the `Command` trait itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandMetadata {
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aliases: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub availability: Option<Vec<CommandAvailability>>,
    #[serde(default)]
    pub is_hidden: bool,
    #[serde(default)]
    pub is_mcp: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argument_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when_to_use: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default)]
    pub disable_model_invocation: bool,
    #[serde(default)]
    pub user_invocable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loaded_from: Option<CommandLoadedFrom>,
    /// TS `kind: 'workflow'`. Rust keeps it as `Option<String>`
    /// because future variants (workflow, skill-script, …) can
    /// be added without breaking serde roundtrip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default)]
    pub immediate: bool,
    #[serde(default)]
    pub is_sensitive: bool,
    /// Plugin source provenance (`"slack@anthropic"` etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_source: Option<String>,
}

impl CommandMetadata {
    /// Create metadata with minimal required fields. Non-required
    /// fields default; bool flags default to `false`.
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            aliases: None,
            availability: None,
            is_hidden: false,
            is_mcp: false,
            argument_hint: None,
            when_to_use: None,
            version: None,
            disable_model_invocation: false,
            user_invocable: true,
            loaded_from: None,
            kind: None,
            immediate: false,
            is_sensitive: false,
            plugin_source: None,
        }
    }
}

/// A command's decision on how to display the result. TS
/// `CommandResultDisplay = 'skip' | 'system' | 'user'`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandResultDisplay {
    /// Don't surface the result at all (TS `'skip'`).
    Skip,
    /// Surface as a system message.
    System,
    /// Surface as a user message (default).
    User,
}

/// What a command invocation produces. Covers the union of TS
/// `PromptCommand` / `LocalCommand` / `LocalJSXCommand` outputs
/// WITHOUT the JSX embedding — Codex Q6 is explicit that JSX
/// belongs in the frontend crate.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CommandOutcome {
    /// TS `PromptCommand`: expand into a message to the model.
    /// `content_blocks` is the already-assembled
    /// `ContentBlockParam[]` (kept as opaque `Value` since
    /// the block types come from the Anthropic SDK).
    Prompt { content_blocks: Vec<Value> },
    /// TS `LocalCommand` text output. Displayed per `display`.
    Text {
        value: String,
        #[serde(default = "default_display")]
        display: CommandResultDisplay,
    },
    /// Silent success — no display, no further action.
    Skip,
    /// Invoke compaction with the enclosed compaction result
    /// shape (TS `{type: 'compact', compactionResult, displayText}`).
    /// `details` is `Value` because `CompactionResult` lives in
    /// `services/compact/compact.ts` with its own type graph.
    Compact {
        details: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        display_text: Option<String>,
    },
    /// A request for UI-side render. The frontend crate owns
    /// interpretation. `spec` is intentionally open
    /// (`Value`) — core cares that the command wants to show
    /// something; it doesn't care what. This replaces TS
    /// `LocalJSXCommand` without embedding JSX in core.
    Render { spec: Value },
}

fn default_display() -> CommandResultDisplay {
    CommandResultDisplay::User
}

#[derive(Debug, Error)]
pub enum CommandError {
    #[error("unknown command: {0}")]
    Unknown(String),
    #[error("argument parse error: {0}")]
    BadArgs(String),
    #[error("execution failed: {0}")]
    Execution(String),
    #[error("{0}")]
    Other(#[from] anyhow::Error),
}

/// Per-invocation context. Lightweight on purpose: commands
/// get the current args + the main-loop options snapshot they
/// need for prompt construction + the session working directory.
/// Anything heavier (host capabilities, app-state handle) is
/// passed through the already-designed `ToolHost` boundary;
/// commands that need state mutation do it via the host too.
#[derive(Debug, Clone)]
pub struct CommandContext<'a> {
    pub args: &'a str,
    pub options: &'a crate::tool_use_context_options::ToolUseContextOptions,
    /// Session working directory. Legacy `CommandContext` (in
    /// `commands::registry`) carries this inline; exposing it
    /// here means adapters can forward it without falling back
    /// to `std::env::current_dir()`, which is process-level and
    /// can disagree with the session's cwd.
    pub working_directory: &'a std::path::Path,
}

/// Unified command interface. Built-in / plugin / skill / MCP
/// commands all implement this — "loaded from where" is
/// `CommandMetadata::loaded_from`, not a type-system split.
#[async_trait]
pub trait Command: Send + Sync {
    /// Pure-data metadata. Cheap to clone; used for `/help`
    /// listing, typeahead, permission checks.
    fn metadata(&self) -> &CommandMetadata;

    /// User-facing display name. TS `CommandBase.userFacingName?`
    /// — default implementation returns the registered name.
    /// Plugin commands that strip a prefix override this.
    fn user_facing_name(&self) -> String {
        self.metadata().name.clone()
    }

    /// Runtime enablement gate. TS `isEnabled?: () => boolean`
    /// with default `true`. Can consult env vars / GrowthBook
    /// / platform-specific checks.
    fn is_enabled(&self) -> bool {
        true
    }

    /// Execute the command. Returns a structured
    /// `CommandOutcome`; the frontend interprets render specs,
    /// the model loop consumes prompt outputs, etc.
    async fn execute(&self, ctx: CommandContext<'_>) -> Result<CommandOutcome, CommandError>;
}

/// Shared command registry. Matches the shape of the existing
/// `claude-tools::ToolRegistry` — a `HashMap<name, Arc<dyn
/// Command>>` — so callers can treat commands and tools
/// symmetrically.
#[derive(Default)]
pub struct CommandRegistry {
    by_name: std::collections::HashMap<String, Arc<dyn Command>>,
    aliases: std::collections::HashMap<String, String>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, cmd: Arc<dyn Command>) {
        let meta = cmd.metadata().clone();
        if let Some(aliases) = &meta.aliases {
            for a in aliases {
                self.aliases.insert(a.clone(), meta.name.clone());
            }
        }
        self.by_name.insert(meta.name, cmd);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Command>> {
        self.by_name
            .get(name)
            .or_else(|| self.aliases.get(name).and_then(|n| self.by_name.get(n)))
            .cloned()
    }

    pub fn all(&self) -> Vec<Arc<dyn Command>> {
        self.by_name.values().cloned().collect()
    }

    /// Filter by availability + `is_enabled()`. Matches TS
    /// `commands.ts:meetsAvailabilityRequirement` usage +
    /// `isCommandEnabled` chain.
    pub fn visible_for(&self, user: CommandAvailability) -> Vec<Arc<dyn Command>> {
        self.all()
            .into_iter()
            .filter(|c| c.is_enabled())
            .filter(|c| {
                c.metadata()
                    .availability
                    .as_ref()
                    .is_none_or(|list| list.contains(&user))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::thinking_config::ThinkingConfig;
    use crate::tool_use_context_options::ToolUseContextOptions;
    use serde_json::json;
    use std::collections::HashMap;

    fn sample_options() -> ToolUseContextOptions {
        ToolUseContextOptions {
            commands: vec![],
            debug: false,
            main_loop_model: "m".into(),
            tools: vec![],
            verbose: false,
            thinking_config: ThinkingConfig::Adaptive,
            mcp_clients: vec![],
            mcp_resources: HashMap::new(),
            is_non_interactive_session: true,
            agent_definitions: Value::Null,
            max_budget_usd: None,
            custom_system_prompt: None,
            append_system_prompt: None,
            query_source: None,
            session_id: None,
        }
    }

    #[test]
    fn metadata_new_sets_user_invocable_default_true() {
        let m = CommandMetadata::new("test", "A test command");
        assert_eq!(m.name, "test");
        assert!(m.user_invocable);
        assert!(!m.is_hidden);
    }

    #[test]
    fn availability_serialises_kebab_case() {
        assert_eq!(
            serde_json::to_value(CommandAvailability::ClaudeAi).unwrap(),
            json!("claude-ai"),
        );
        assert_eq!(
            serde_json::to_value(CommandAvailability::Console).unwrap(),
            json!("console"),
        );
    }

    #[test]
    fn outcome_prompt_serialises_with_kind_tag() {
        let out = CommandOutcome::Prompt {
            content_blocks: vec![json!({"type": "text", "text": "hello"})],
        };
        let v = serde_json::to_value(&out).unwrap();
        assert_eq!(v["kind"], "prompt");
        assert_eq!(v["content_blocks"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn outcome_text_uses_user_display_by_default() {
        // deserialise a text outcome that omits `display`.
        let v = json!({ "kind": "text", "value": "ok" });
        let out: CommandOutcome = serde_json::from_value(v).unwrap();
        match out {
            CommandOutcome::Text { value, display } => {
                assert_eq!(value, "ok");
                assert!(matches!(display, CommandResultDisplay::User));
            }
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn outcome_render_keeps_spec_opaque() {
        let out = CommandOutcome::Render {
            spec: json!({"widget": "selector", "items": [1, 2, 3]}),
        };
        let v = serde_json::to_value(&out).unwrap();
        assert_eq!(v["kind"], "render");
        assert_eq!(v["spec"]["widget"], "selector");
    }

    struct StaticTextCommand(CommandMetadata);

    #[async_trait]
    impl Command for StaticTextCommand {
        fn metadata(&self) -> &CommandMetadata {
            &self.0
        }
        async fn execute(&self, ctx: CommandContext<'_>) -> Result<CommandOutcome, CommandError> {
            Ok(CommandOutcome::Text {
                value: format!("{}: {}", self.0.name, ctx.args),
                display: CommandResultDisplay::System,
            })
        }
    }

    #[tokio::test]
    async fn execute_flows_through_context() {
        let cmd = StaticTextCommand(CommandMetadata::new("ping", "Ping check"));
        let opts = sample_options();
        let out = cmd
            .execute(CommandContext {
                args: "hello",
                options: &opts,
                working_directory: std::path::Path::new("."),
            })
            .await
            .unwrap();
        match out {
            CommandOutcome::Text { value, display } => {
                assert_eq!(value, "ping: hello");
                assert!(matches!(display, CommandResultDisplay::System));
            }
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn registry_register_and_get_by_name() {
        let mut r = CommandRegistry::new();
        r.register(Arc::new(StaticTextCommand(CommandMetadata::new(
            "one", "First",
        ))));
        assert!(r.get("one").is_some());
        assert!(r.get("missing").is_none());
    }

    #[test]
    fn registry_alias_lookup() {
        let mut meta = CommandMetadata::new("memory", "Edit memory");
        meta.aliases = Some(vec!["mem".into(), "m".into()]);
        let mut r = CommandRegistry::new();
        r.register(Arc::new(StaticTextCommand(meta)));
        assert!(r.get("memory").is_some());
        assert!(r.get("mem").is_some());
        assert!(r.get("m").is_some());
        assert!(r.get("notanalias").is_none());
    }

    #[test]
    fn registry_visible_for_filters_by_availability() {
        let mut claude_only = CommandMetadata::new("bug", "Report a bug");
        claude_only.availability = Some(vec![CommandAvailability::ClaudeAi]);
        let any = CommandMetadata::new("help", "Show help");
        // `any` has no availability → visible to everyone.

        let mut r = CommandRegistry::new();
        r.register(Arc::new(StaticTextCommand(claude_only)));
        r.register(Arc::new(StaticTextCommand(any)));

        let console_user_visible = r.visible_for(CommandAvailability::Console);
        assert_eq!(console_user_visible.len(), 1);
        assert_eq!(console_user_visible[0].metadata().name, "help");

        let claude_user_visible = r.visible_for(CommandAvailability::ClaudeAi);
        assert_eq!(claude_user_visible.len(), 2);
    }

    #[test]
    fn registry_visible_for_respects_is_enabled() {
        struct DisabledCommand(CommandMetadata);
        #[async_trait]
        impl Command for DisabledCommand {
            fn metadata(&self) -> &CommandMetadata {
                &self.0
            }
            fn is_enabled(&self) -> bool {
                false
            }
            async fn execute(&self, _: CommandContext<'_>) -> Result<CommandOutcome, CommandError> {
                Ok(CommandOutcome::Skip)
            }
        }
        let mut r = CommandRegistry::new();
        r.register(Arc::new(DisabledCommand(CommandMetadata::new(
            "gone", "removed",
        ))));
        assert_eq!(r.visible_for(CommandAvailability::Console).len(), 0);
        // But direct lookup still finds it.
        assert!(r.get("gone").is_some());
    }

    #[test]
    fn user_facing_name_defaults_to_name() {
        let cmd = StaticTextCommand(CommandMetadata::new("list", "List things"));
        assert_eq!(cmd.user_facing_name(), "list");
    }

    #[test]
    fn metadata_roundtrips_camel_and_snake() {
        let meta = CommandMetadata {
            name: "x".into(),
            description: "d".into(),
            aliases: Some(vec!["y".into()]),
            availability: Some(vec![CommandAvailability::ClaudeAi]),
            is_hidden: true,
            is_mcp: false,
            argument_hint: Some("<arg>".into()),
            when_to_use: None,
            version: Some("1.0".into()),
            disable_model_invocation: false,
            user_invocable: true,
            loaded_from: Some(CommandLoadedFrom::Plugin),
            kind: Some("workflow".into()),
            immediate: false,
            is_sensitive: false,
            plugin_source: Some("slack@anthropic".into()),
        };
        let v = serde_json::to_value(&meta).unwrap();
        let back: CommandMetadata = serde_json::from_value(v).unwrap();
        assert_eq!(back.name, meta.name);
        assert_eq!(back.aliases, meta.aliases);
        assert_eq!(back.loaded_from, meta.loaded_from);
        assert_eq!(back.plugin_source, meta.plugin_source);
    }
}
