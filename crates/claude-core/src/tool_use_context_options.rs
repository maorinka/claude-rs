//! Pure-data subset of `ToolUseContext.options`.
//!
//! Port of TS `Tool.ts:158-179` — the `options` field of
//! `ToolUseContext`. This is the slice of tool-invocation context
//! that's plain data (no callbacks, no React setters, no abort
//! controllers) and therefore portable as a Rust struct.
//!
//! Scope
//! =====
//! Only the `.options` sub-object is ported here. The outer
//! `ToolUseContext` struct has ~40 additional fields, most of them
//! function-typed callbacks (`setAppState`, `setResponseLength`,
//! `updateFileHistoryState`, `requestPrompt`, `handleElicitation`,
//! etc.) that are runtime behaviour, not data. Those belong behind
//! trait-object adapters at the consumer layer (see
//! `claude-tools::registry::ToolUseContext` for the Rust
//! equivalent's minimal form).
//!
//! Fields with missing underlying types
//! ====================================
//! Several `.options` fields reference types whose TS source is
//! not ported (or not portable as plain data):
//!
//! - `commands: Command[]` — TS `Command` is a 3-variant union
//!   with async `load()` / `call()` callbacks + React JSX. Not
//!   portable as plain data.
//! - `tools: Tools` — TS `Tools` is `readonly Tool[]` where `Tool`
//!   has async `call` + render methods. The Rust equivalent is
//!   the `claude-tools::ToolExecutor` trait; the list lives as
//!   `Vec<Arc<dyn ToolExecutor>>` at call sites.
//! - `agentDefinitions: AgentDefinitionsResult` — `AgentDefinition`
//!   variants carry `getSystemPrompt` callbacks.
//! - `refreshTools: () => Tools` — a caller-supplied fetcher.
//!
//! These fields are represented here as `serde_json::Value` or
//! `Vec<serde_json::Value>` — callers serialise their runtime
//! structures before assembling the options struct. That keeps
//! the data layer honest about what's opaque without faking type
//! fidelity.

use crate::mcp_server_connection::McpServerConnection;
use crate::thinking_config::ThinkingConfig;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolUseContextOptions {
    /// TS `commands: Command[]` — opaque because TS Command has
    /// async-callback variants.
    #[serde(default)]
    pub commands: Vec<Value>,

    pub debug: bool,

    /// The model identifier used by the main loop this turn.
    #[serde(rename = "mainLoopModel")]
    pub main_loop_model: String,

    /// TS `tools: Tools` — opaque because TS Tool is a trait-like
    /// interface with async methods + render methods. Serialise
    /// tool definitions (name + description + input_schema) here
    /// if the caller needs wire representation.
    #[serde(default)]
    pub tools: Vec<Value>,

    pub verbose: bool,

    /// Already-ported thinking config.
    #[serde(rename = "thinkingConfig")]
    pub thinking_config: ThinkingConfig,

    /// Live MCP server connection states. Uses the already-ported
    /// `McpServerConnection` enum.
    #[serde(rename = "mcpClients", default)]
    pub mcp_clients: Vec<McpServerConnection>,

    /// TS `mcpResources: Record<string, ServerResource[]>`. MCP
    /// resource lists per server name; `Value` because MCP
    /// `Resource` comes from the MCP SDK and is kept SDK-version-
    /// agnostic here.
    #[serde(rename = "mcpResources", default)]
    pub mcp_resources: HashMap<String, Vec<Value>>,

    /// Print mode / SDK caller flag.
    #[serde(rename = "isNonInteractiveSession")]
    pub is_non_interactive_session: bool,

    /// TS `AgentDefinitionsResult` — opaque. Callers serialise
    /// their runtime list before assembling options.
    #[serde(rename = "agentDefinitions", default)]
    pub agent_definitions: Value,

    /// Budget cap in USD; `None` → unlimited.
    #[serde(rename = "maxBudgetUsd", default, skip_serializing_if = "Option::is_none")]
    pub max_budget_usd: Option<f64>,

    /// Overrides the default system prompt entirely.
    #[serde(
        rename = "customSystemPrompt",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub custom_system_prompt: Option<String>,

    /// Appended AFTER the main system prompt.
    #[serde(
        rename = "appendSystemPrompt",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub append_system_prompt: Option<String>,

    /// Optional analytics query-source override.
    #[serde(
        rename = "querySource",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub query_source: Option<String>,
}

impl ToolUseContextOptions {
    /// Minimal constructor with sensible defaults for non-UI
    /// callers.
    pub fn minimal(main_loop_model: impl Into<String>) -> Self {
        Self {
            commands: Vec::new(),
            debug: false,
            main_loop_model: main_loop_model.into(),
            tools: Vec::new(),
            verbose: false,
            thinking_config: ThinkingConfig::Adaptive,
            mcp_clients: Vec::new(),
            mcp_resources: HashMap::new(),
            is_non_interactive_session: true,
            agent_definitions: Value::Null,
            max_budget_usd: None,
            custom_system_prompt: None,
            append_system_prompt: None,
            query_source: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn minimal_construction() {
        let opts = ToolUseContextOptions::minimal("claude-opus-4-7");
        assert_eq!(opts.main_loop_model, "claude-opus-4-7");
        assert!(opts.is_non_interactive_session);
        assert!(!opts.debug);
        assert!(matches!(opts.thinking_config, ThinkingConfig::Adaptive));
    }

    #[test]
    fn serialises_camel_case_every_field() {
        let opts = ToolUseContextOptions {
            commands: vec![json!({"name": "test"})],
            debug: true,
            main_loop_model: "m".into(),
            tools: vec![],
            verbose: true,
            thinking_config: ThinkingConfig::Disabled,
            mcp_clients: vec![],
            mcp_resources: HashMap::new(),
            is_non_interactive_session: false,
            agent_definitions: json!({"activeAgents": []}),
            max_budget_usd: Some(5.0),
            custom_system_prompt: Some("custom".into()),
            append_system_prompt: None,
            query_source: Some("repl_main_thread".into()),
        };
        let v = serde_json::to_value(&opts).unwrap();
        for k in [
            "commands",
            "debug",
            "mainLoopModel",
            "tools",
            "verbose",
            "thinkingConfig",
            "mcpClients",
            "mcpResources",
            "isNonInteractiveSession",
            "agentDefinitions",
            "maxBudgetUsd",
            "customSystemPrompt",
            "querySource",
        ] {
            assert!(v.get(k).is_some(), "missing key: {k}");
        }
        // `appendSystemPrompt` should be omitted when None.
        assert!(v.as_object().unwrap().get("appendSystemPrompt").is_none());
    }

    #[test]
    fn deserialises_from_ts_wire_shape() {
        let wire = json!({
            "commands": [],
            "debug": false,
            "mainLoopModel": "claude-opus-4-7",
            "tools": [],
            "verbose": false,
            "thinkingConfig": { "type": "adaptive" },
            "mcpClients": [],
            "mcpResources": {},
            "isNonInteractiveSession": true,
            "agentDefinitions": null,
            "customSystemPrompt": "override",
            "appendSystemPrompt": "extra"
        });
        let opts: ToolUseContextOptions = serde_json::from_value(wire).unwrap();
        assert_eq!(opts.main_loop_model, "claude-opus-4-7");
        assert_eq!(opts.custom_system_prompt.as_deref(), Some("override"));
        assert_eq!(opts.append_system_prompt.as_deref(), Some("extra"));
    }

    #[test]
    fn thinking_config_roundtrip_through_options() {
        let opts = ToolUseContextOptions {
            thinking_config: ThinkingConfig::Enabled { budget_tokens: 16_000 },
            ..ToolUseContextOptions::minimal("m")
        };
        let v = serde_json::to_value(&opts).unwrap();
        assert_eq!(
            v["thinkingConfig"],
            json!({ "type": "enabled", "budgetTokens": 16000 })
        );
        let back: ToolUseContextOptions = serde_json::from_value(v).unwrap();
        assert!(matches!(
            back.thinking_config,
            ThinkingConfig::Enabled { budget_tokens: 16_000 }
        ));
    }

    #[test]
    fn optional_fields_omitted_when_none() {
        let opts = ToolUseContextOptions::minimal("m");
        let v = serde_json::to_value(&opts).unwrap();
        let obj = v.as_object().unwrap();
        assert!(obj.get("maxBudgetUsd").is_none());
        assert!(obj.get("customSystemPrompt").is_none());
        assert!(obj.get("appendSystemPrompt").is_none());
        assert!(obj.get("querySource").is_none());
    }

    #[test]
    fn default_deserialisation_for_missing_collections() {
        // TS sends empty arrays / objects; the Rust port should
        // accept either missing or empty.
        let minimal = json!({
            "debug": false,
            "mainLoopModel": "m",
            "verbose": false,
            "thinkingConfig": { "type": "adaptive" },
            "isNonInteractiveSession": true
        });
        let opts: ToolUseContextOptions = serde_json::from_value(minimal).unwrap();
        assert!(opts.commands.is_empty());
        assert!(opts.tools.is_empty());
        assert!(opts.mcp_clients.is_empty());
        assert!(opts.mcp_resources.is_empty());
    }
}
