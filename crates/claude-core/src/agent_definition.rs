//! Agent definition type — metadata + system-prompt source.
//!
//! Port of TS `tools/AgentTool/loadAgentsDir.ts:162` (the
//! `AgentDefinition` union) plus the shared `BaseAgentDefinition`
//! fields.
//!
//! **Step 3a** of the design rollout (Codex CLI gpt-5.4,
//! 2026-04-20). Codex Q5 verdict implemented here:
//!
//! > "Use **(b) enum with `PromptSource = Static(String) |
//! > Dynamic(Arc<dyn Fn(&ToolUseContextOptions) -> String + Send +
//! > Sync>)`**, not full trait objects and not pre-resolution at
//! > load time. This matches the real requirement: most agents
//! > are static, built-ins need launch-time context, and the
//! > dynamic surface area is tiny. Do not pass full
//! > `ToolUseContext`; the TS built-in callback only needs the
//! > `options` subset, and keeping the callback input narrow
//! > prevents agent definitions from silently depending on
//! > runtime host/UI state."
//!
//! The `Dynamic` callback's input is therefore
//! `&ToolUseContextOptions` (already ported in
//! `tool_use_context_options`), not full `ToolUseContext`.
//!
//! # Source discriminator
//!
//! TS splits `AgentDefinition` into three types
//! (`BuiltInAgentDefinition` / `CustomAgentDefinition` /
//! `PluginAgentDefinition`) via the `source` field. Rust uses
//! one struct + a flat `AgentSource` enum; the variant just tags
//! where the agent was loaded from. This matches TS's
//! `isBuiltInAgent` / `isCustomAgent` / `isPluginAgent` guards,
//! which all test `source === 'built-in'` / `source === 'plugin'`.

use crate::tool_use_context_options::ToolUseContextOptions;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Where the agent definition was loaded from. Matches TS
/// `source` on `BaseAgentDefinition` (`'built-in' | 'plugin' |
/// SettingSource`). `SettingSource` itself is
/// `'userSettings' | 'projectSettings' | 'localSettings' |
/// 'flagSettings' | 'policySettings'`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentSource {
    /// `'built-in'` on the wire — we keep the kebab spelling
    /// via manual rename.
    #[serde(rename = "built-in")]
    BuiltIn,
    #[serde(rename = "plugin")]
    Plugin,
    #[serde(rename = "userSettings")]
    UserSettings,
    #[serde(rename = "projectSettings")]
    ProjectSettings,
    #[serde(rename = "localSettings")]
    LocalSettings,
    #[serde(rename = "flagSettings")]
    FlagSettings,
    #[serde(rename = "policySettings")]
    PolicySettings,
}

impl AgentSource {
    /// Matches TS `isCustomAgent`: any `SettingSource` variant
    /// (neither built-in nor plugin).
    pub fn is_custom(&self) -> bool {
        matches!(
            self,
            Self::UserSettings
                | Self::ProjectSettings
                | Self::LocalSettings
                | Self::FlagSettings
                | Self::PolicySettings
        )
    }

    pub fn is_built_in(&self) -> bool {
        matches!(self, Self::BuiltIn)
    }

    pub fn is_plugin(&self) -> bool {
        matches!(self, Self::Plugin)
    }
}

/// Dynamic-prompt callback trait alias. Takes a NARROW input
/// (`&ToolUseContextOptions`, the data-only `.options` subset),
/// not full `ToolUseContext` — per Codex Q5, this prevents
/// definitions from silently depending on host/UI state.
pub type DynamicPromptFn = Arc<dyn Fn(&ToolUseContextOptions) -> String + Send + Sync>;

/// System-prompt provider. TS built-in agents take the
/// `toolUseContext.options` subset at resolve time; custom /
/// plugin agents close over the loaded markdown content and
/// return it directly. The enum captures both cases.
#[derive(Clone)]
pub enum PromptSource {
    /// Resolved at load time. Matches TS `CustomAgentDefinition`
    /// and `PluginAgentDefinition` whose `getSystemPrompt: () =>
    /// string` closes over a static prompt string.
    Static(String),
    /// Resolved at call time against the current
    /// `ToolUseContextOptions`. Matches TS
    /// `BuiltInAgentDefinition.getSystemPrompt({ toolUseContext:
    /// Pick<ToolUseContext, 'options'> })`.
    Dynamic(DynamicPromptFn),
}

impl std::fmt::Debug for PromptSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Static(s) => {
                let preview: String = s.chars().take(40).collect();
                write!(
                    f,
                    "Static({preview:?}{})",
                    if s.len() > 40 { "..." } else { "" }
                )
            }
            Self::Dynamic(_) => write!(f, "Dynamic(<closure>)"),
        }
    }
}

impl PromptSource {
    /// Resolve the prompt string. Static → clone, Dynamic →
    /// invoke the closure with the supplied options.
    pub fn resolve(&self, options: &ToolUseContextOptions) -> String {
        match self {
            Self::Static(s) => s.clone(),
            Self::Dynamic(f) => f(options),
        }
    }
}

/// Shared fields across all three TS variants
/// (`BaseAgentDefinition` at `loadAgentsDir.ts:106-134`). Not
/// every field maps 1:1 — types like `AgentColorName`,
/// `EffortValue`, `PermissionMode`, `HooksSettings`,
/// `AgentMcpServerSpec` have their own nested ports. Where
/// unported, stored as `Option<String>` or `Value`. Callers
/// that need typed access should port those sub-types at the
/// consumer layer.
#[derive(Debug, Clone)]
pub struct AgentDefinition {
    pub agent_type: String,
    pub when_to_use: String,
    pub source: AgentSource,
    pub prompt: PromptSource,
    pub tools: Option<Vec<String>>,
    pub disallowed_tools: Option<Vec<String>>,
    /// Comma-separated skill names from the frontmatter,
    /// pre-split. TS `skills?: string[]`.
    pub skills: Option<Vec<String>>,
    /// AgentColorName as opaque string ("red", "blue", "ant-
    /// only-gold", etc. — the enum lives elsewhere).
    pub color: Option<String>,
    /// Model id override. TS has `inherit` as a sentinel.
    pub model: Option<String>,
    /// `EffortValue` as opaque string — TS union of several
    /// numeric/label variants; consumer converts if needed.
    pub effort: Option<String>,
    /// `PermissionMode` as opaque string — resolves via the
    /// ported `PermissionMode` enum at use site.
    pub permission_mode: Option<String>,
    /// Maximum agentic turns before stopping.
    pub max_turns: Option<u32>,
    /// Source filename (without `.md`) for user/project agents.
    pub filename: Option<String>,
    /// Base directory for resource resolution
    /// (CLAUDE_PLUGIN_ROOT for plugin agents, cwd for custom).
    pub base_dir: Option<String>,
    /// Short message re-injected at every user turn
    /// (TS `criticalSystemReminder_EXPERIMENTAL`).
    pub critical_system_reminder: Option<String>,
    /// MCP server name patterns required by this agent.
    pub required_mcp_servers: Option<Vec<String>>,
    /// Always-background task flag (TS `background?: boolean`).
    pub background: Option<bool>,
    /// Prepended to the first user turn.
    pub initial_prompt: Option<String>,
    /// Persistent memory scope: `"user"` / `"project"` /
    /// `"local"`. TS `AgentMemoryScope` — kept as opaque
    /// string.
    pub memory: Option<String>,
    /// Isolation level: `"worktree"` (external) or `"remote"`
    /// (ant-only).
    pub isolation: Option<String>,
    /// Opt out of hierarchical CLAUDE.md injection.
    pub omit_claude_md: Option<bool>,
    /// For plugin agents: the providing plugin's source
    /// (e.g. `"slack@anthropic"`).
    pub plugin_source: Option<String>,
}

/// Bundle returned by the agent loader. TS
/// `AgentDefinitionsResult` at `loadAgentsDir.ts:186-191`.
#[derive(Debug, Clone, Default)]
pub struct AgentDefinitionsResult {
    pub active_agents: Vec<AgentDefinition>,
    pub all_agents: Vec<AgentDefinition>,
    pub failed_files: Vec<FailedAgentFile>,
    pub allowed_agent_types: Option<Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct FailedAgentFile {
    pub path: String,
    pub error: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::thinking_config::ThinkingConfig;
    use serde_json::json;
    use std::collections::HashMap;

    fn sample_options() -> ToolUseContextOptions {
        ToolUseContextOptions {
            commands: vec![],
            debug: false,
            main_loop_model: "claude-opus-4-7".into(),
            tools: vec![],
            verbose: false,
            thinking_config: ThinkingConfig::Adaptive,
            mcp_clients: vec![],
            mcp_resources: HashMap::new(),
            is_non_interactive_session: true,
            agent_definitions: serde_json::Value::Null,
            max_budget_usd: None,
            custom_system_prompt: None,
            append_system_prompt: None,
            query_source: None,
            session_id: None,
        }
    }

    #[test]
    fn agent_source_predicates() {
        assert!(AgentSource::BuiltIn.is_built_in());
        assert!(!AgentSource::BuiltIn.is_custom());
        assert!(!AgentSource::BuiltIn.is_plugin());

        assert!(AgentSource::Plugin.is_plugin());
        assert!(!AgentSource::Plugin.is_custom());

        for s in [
            AgentSource::UserSettings,
            AgentSource::ProjectSettings,
            AgentSource::LocalSettings,
            AgentSource::FlagSettings,
            AgentSource::PolicySettings,
        ] {
            assert!(s.is_custom(), "{s:?} should be custom");
            assert!(!s.is_built_in());
            assert!(!s.is_plugin());
        }
    }

    #[test]
    fn agent_source_serialises_ts_wire() {
        assert_eq!(
            serde_json::to_value(AgentSource::BuiltIn).unwrap(),
            json!("built-in"),
        );
        assert_eq!(
            serde_json::to_value(AgentSource::UserSettings).unwrap(),
            json!("userSettings"),
        );
    }

    #[test]
    fn prompt_source_static_resolves_unchanged() {
        let ps = PromptSource::Static("hello world".into());
        let opts = sample_options();
        assert_eq!(ps.resolve(&opts), "hello world");
    }

    #[test]
    fn prompt_source_dynamic_receives_options() {
        // The dynamic callback sees the options subset, NOT full
        // ToolUseContext — this is the Q5 contract. Prove it by
        // returning a string built from `main_loop_model`.
        let ps = PromptSource::Dynamic(Arc::new(|opts: &ToolUseContextOptions| {
            format!("model={}", opts.main_loop_model)
        }));
        let opts = sample_options();
        assert_eq!(ps.resolve(&opts), "model=claude-opus-4-7");
    }

    #[test]
    fn prompt_source_debug_truncates_long_static() {
        let long = "x".repeat(200);
        let ps = PromptSource::Static(long);
        let dbg = format!("{ps:?}");
        assert!(dbg.contains("Static"));
        assert!(dbg.ends_with("...)"), "expected truncation marker: {dbg}");
    }

    #[test]
    fn prompt_source_debug_dynamic_redacts_closure() {
        let ps = PromptSource::Dynamic(Arc::new(|_| "x".into()));
        let dbg = format!("{ps:?}");
        assert_eq!(dbg, "Dynamic(<closure>)");
    }

    #[test]
    fn agent_definition_clone() {
        // Ensure the AgentDefinition struct Clones cleanly (it
        // contains an Arc-backed closure in PromptSource::Dynamic
        // — wrong cloning would need explicit impl).
        let a = AgentDefinition {
            agent_type: "researcher".into(),
            when_to_use: "for deep research".into(),
            source: AgentSource::UserSettings,
            prompt: PromptSource::Dynamic(Arc::new(|o| o.main_loop_model.clone())),
            tools: Some(vec!["Read".into()]),
            disallowed_tools: None,
            skills: None,
            color: Some("blue".into()),
            model: None,
            effort: None,
            permission_mode: Some("default".into()),
            max_turns: Some(20),
            filename: Some("researcher".into()),
            base_dir: None,
            critical_system_reminder: None,
            required_mcp_servers: None,
            background: Some(false),
            initial_prompt: None,
            memory: Some("project".into()),
            isolation: None,
            omit_claude_md: Some(false),
            plugin_source: None,
        };
        let b = a.clone();
        assert_eq!(a.agent_type, b.agent_type);
        assert_eq!(a.max_turns, b.max_turns);
        assert_eq!(a.color, b.color);
        // Both clones resolve the same dynamic prompt.
        let opts = sample_options();
        assert_eq!(a.prompt.resolve(&opts), b.prompt.resolve(&opts));
    }

    #[test]
    fn definitions_result_default_empty() {
        let r = AgentDefinitionsResult::default();
        assert!(r.active_agents.is_empty());
        assert!(r.all_agents.is_empty());
        assert!(r.failed_files.is_empty());
        assert!(r.allowed_agent_types.is_none());
    }
}
