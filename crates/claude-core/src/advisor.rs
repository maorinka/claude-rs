//! Advisor tool feature-gating, type shapes, and the tool-instructions
//! prompt.
//!
//! Ports `src/utils/advisor.ts`. The advisor is a server-side tool backed
//! by a stronger reviewer model; when it's called, the full conversation
//! history is forwarded server-side, so the Rust client only needs:
//!
//!   1. Type shapes to recognise advisor blocks in responses
//!      (`AdvisorBlock` + `is_advisor_block`).
//!   2. Feature-gate predicates (`is_advisor_enabled`,
//!      `can_user_configure_advisor`).
//!   3. Model-compatibility checks (`model_supports_advisor`,
//!      `is_valid_advisor_model`).
//!   4. The `ADVISOR_TOOL_INSTRUCTIONS` prompt text, injected into the
//!      system prompt when the feature is enabled.
//!
//! Growthbook / initial-settings wiring
//! =====================================
//! TS reaches the growthbook client via
//! `getFeatureValue_CACHED_MAY_BE_STALE<AdvisorConfig>('tengu_sage_compass', {})`
//! and `getInitialSettings().advisorModel`. Rust doesn't yet have those
//! sinks wired on the hot path, so `AdvisorConfig` is passed in
//! explicitly by callers that do have a growthbook/settings view. The
//! pure predicates here (model matching, env gates, first-party beta
//! gate) are fully ported and don't need that wiring.
//!
//! Deferred helpers
//! ================
//! Two TS helpers are intentionally not ported here; each is a thin
//! one-liner whose cost is paying for a dependency graph we haven't
//! wired:
//!   - `getInitialAdvisorSetting()` at `advisor.ts:108-113` — reads
//!     `getInitialSettings().advisorModel`. Port once `settings/` is
//!     wired; the logic is `is_advisor_enabled(cfg).then(|| settings.advisor_model).flatten()`.
//!   - `getAdvisorUsage(usage)` at `advisor.ts:115-128` — filters a
//!     `BetaUsage.iterations` array for `type === 'advisor_message'`.
//!     Port once the SDK `BetaUsage` shape lands on the Rust side.

use crate::errors_util::is_env_truthy;
use crate::privacy_level::{get_api_provider, ApiProvider};
use crate::user_type;

/// The growthbook `tengu_sage_compass` config shape. Populated by the
/// caller from their growthbook view. TS at `advisor.ts:46-51`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AdvisorConfig {
    pub enabled: Option<bool>,
    pub can_user_configure: Option<bool>,
    pub base_model: Option<String>,
    pub advisor_model: Option<String>,
}

/// Shape of an `advisor` `server_tool_use` block in a message.
/// TS: `AdvisorServerToolUseBlock` at `advisor.ts:9-14`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvisorServerToolUseBlock {
    pub id: String,
    /// Free-form JSON input — the server forwards the conversation
    /// automatically, so the tool takes no user-provided arguments in
    /// practice, but the shape preserves whatever the API returns.
    pub input: serde_json::Value,
}

/// Shape of the three possible `advisor_tool_result` payloads.
/// TS: `AdvisorToolResultBlock` at `advisor.ts:16-32`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdvisorResultContent {
    /// The advisor's plain-text recommendation.
    Result { text: String },
    /// Redacted / encrypted payload we do not display to the model.
    Redacted { encrypted_content: String },
    /// Advisor errored — surfaced by error_code, not text.
    Error { error_code: String },
}

/// A complete `advisor_tool_result` block: which `server_tool_use.id`
/// it answers plus the payload. TS: `advisor.ts:16-32`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvisorToolResultBlock {
    pub tool_use_id: String,
    pub content: AdvisorResultContent,
}

/// Either flavour of advisor block. TS: `advisor.ts:34`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdvisorBlock {
    ServerToolUse(AdvisorServerToolUseBlock),
    ToolResult(AdvisorToolResultBlock),
}

/// Predicate that matches the two advisor block shapes against the
/// two raw fields the TS type-guard inspects (`type` and optional
/// `name`). Byte-for-byte port of `isAdvisorBlock` at `advisor.ts:36-44`.
///
/// Callers pass the raw strings they've parsed off the content block:
/// - `type_field`: the `"type"` string (`"server_tool_use"` or
///   `"advisor_tool_result"`).
/// - `name_field`: for `server_tool_use` blocks, the `"name"` field
///   (only `"advisor"` matches); otherwise any value including `None`.
pub fn is_advisor_block(type_field: &str, name_field: Option<&str>) -> bool {
    type_field == "advisor_tool_result"
        || (type_field == "server_tool_use" && name_field == Some("advisor"))
}

/// The advisor beta header is first-party-only — Bedrock and Vertex
/// return 400 if it's sent. Port of `shouldIncludeFirstPartyOnlyBetas`
/// from `betas.ts:215-220`.
///
/// This is not declared in `betas.rs` yet on the Rust side, so we
/// inline the gate here. Keeping it colocated with the only current
/// caller avoids a partial-port module. Move it if a second caller
/// appears.
fn should_include_first_party_only_betas() -> bool {
    let p = get_api_provider();
    let is_first_party_or_foundry = p == ApiProvider::FirstParty || p == ApiProvider::Foundry;
    is_first_party_or_foundry && !is_env_truthy("CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS")
}

/// Is the advisor feature enabled for this session? Three gates, all
/// must pass:
///   1. `CLAUDE_CODE_DISABLE_ADVISOR_TOOL` env kill-switch off.
///   2. First-party beta eligibility (provider is firstParty/foundry
///      and experimental betas not disabled).
///   3. Growthbook `tengu_sage_compass.enabled === true`.
///
/// Port of `isAdvisorEnabled` at `advisor.ts:60-69`. Caller supplies
/// the growthbook config; see module docs for why.
pub fn is_advisor_enabled(config: &AdvisorConfig) -> bool {
    if is_env_truthy("CLAUDE_CODE_DISABLE_ADVISOR_TOOL") {
        return false;
    }
    if !should_include_first_party_only_betas() {
        return false;
    }
    config.enabled.unwrap_or(false)
}

/// Can the user override the advisor model themselves (vs. the
/// experiment dictating it)? Requires the feature to be enabled AND
/// the growthbook config's `canUserConfigure` to be true. TS at
/// `advisor.ts:71-73`.
pub fn can_user_configure_advisor(config: &AdvisorConfig) -> bool {
    is_advisor_enabled(config) && config.can_user_configure.unwrap_or(false)
}

/// Experiment-dictated (baseModel, advisorModel) pair — returned only
/// when the feature is on, the user is NOT configuring manually, and
/// both models are non-empty. TS at `advisor.ts:75-85`.
pub fn get_experiment_advisor_models(config: &AdvisorConfig) -> Option<(String, String)> {
    if !is_advisor_enabled(config) || can_user_configure_advisor(config) {
        return None;
    }
    match (&config.base_model, &config.advisor_model) {
        (Some(base), Some(adv)) if !base.is_empty() && !adv.is_empty() => {
            Some((base.clone(), adv.clone()))
        }
        _ => None,
    }
}

/// Does this model support *calling* the advisor tool (i.e. can it
/// serve as the main-loop model while an advisor is attached)?
///
/// TS `modelSupportsAdvisor` at `advisor.ts:89-96` matches opus-4-6,
/// sonnet-4-6, or any model when `USER_TYPE === 'ant'`. The match is
/// case-insensitive substring.
///
/// NOTE: the TS "@[MODEL LAUNCH]" marker signals that this list must
/// be extended when a new Claude model lands. If TS gains a model
/// here, Rust must mirror or advisor silently drops out of the tool
/// set for that model.
pub fn model_supports_advisor(model: &str) -> bool {
    let m = model.to_ascii_lowercase();
    m.contains("opus-4-6") || m.contains("sonnet-4-6") || user_type::is_ant()
}

/// Is this model eligible to *act as* an advisor (the server-side
/// reviewer)? Same match rules as `model_supports_advisor` today, but
/// kept as a separate predicate to match the TS split — the two lists
/// can diverge on future model launches. TS at `advisor.ts:99-106`.
pub fn is_valid_advisor_model(model: &str) -> bool {
    let m = model.to_ascii_lowercase();
    m.contains("opus-4-6") || m.contains("sonnet-4-6") || user_type::is_ant()
}

/// System-prompt fragment injected when the advisor feature is
/// enabled. Verbatim port of `ADVISOR_TOOL_INSTRUCTIONS` at
/// `advisor.ts:130-145`.
pub const ADVISOR_TOOL_INSTRUCTIONS: &str = r#"# Advisor Tool

You have access to an `advisor` tool backed by a stronger reviewer model. It takes NO parameters -- when you call it, your entire conversation history is automatically forwarded. The advisor sees the task, every tool call you've made, every result you've seen.

Call advisor BEFORE substantive work -- before writing code, before committing to an interpretation, before building on an assumption. If the task requires orientation first (finding files, reading code, seeing what's there), do that, then call advisor. Orientation is not substantive work. Writing, editing, and declaring an answer are.

Also call advisor:
- When you believe the task is complete. BEFORE this call, make your deliverable durable: write the file, stage the change, save the result. The advisor call takes time; if the session ends during it, a durable result persists and an unwritten one doesn't.
- When stuck -- errors recurring, approach not converging, results that don't fit.
- When considering a change of approach.

On tasks longer than a few steps, call advisor at least once before committing to an approach and once before declaring done. On short reactive tasks where the next action is dictated by tool output you just read, you don't need to keep calling -- the advisor adds most of its value on the first call, before the approach crystallizes.

Give the advice serious weight. If you follow a step and it fails empirically, or you have primary-source evidence that contradicts a specific claim (the file says X, the code does Y), adapt. A passing self-test is not evidence the advice is wrong -- it's evidence your test doesn't check what the advice is checking.

If you've already retrieved data pointing one way and the advisor points another: don't silently switch. Surface the conflict in one more advisor call -- "I found X, you suggest Y, which constraint breaks the tie?" The advisor saw your evidence but may have underweighted it; a reconcile call is cheaper than committing to the wrong branch."#;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Environment mutation is global; serialise env-touching tests.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct ScopedEnv {
        _guard: std::sync::MutexGuard<'static, ()>,
        keys: Vec<String>,
    }

    impl ScopedEnv {
        fn new() -> Self {
            let guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
            Self {
                _guard: guard,
                keys: Vec::new(),
            }
        }
        fn set(&mut self, k: &str, v: &str) {
            std::env::set_var(k, v);
            self.keys.push(k.to_string());
        }
    }

    impl Drop for ScopedEnv {
        fn drop(&mut self) {
            for k in &self.keys {
                std::env::remove_var(k);
            }
        }
    }

    #[test]
    fn is_advisor_block_matches_result_and_server_tool_use() {
        assert!(is_advisor_block("advisor_tool_result", None));
        assert!(is_advisor_block("advisor_tool_result", Some("anything")));
        assert!(is_advisor_block("server_tool_use", Some("advisor")));
    }

    #[test]
    fn is_advisor_block_rejects_other_server_tools() {
        // Another server-tool name must NOT match.
        assert!(!is_advisor_block("server_tool_use", Some("web_search")));
        // server_tool_use without a name doesn't match.
        assert!(!is_advisor_block("server_tool_use", None));
        // Arbitrary types don't match.
        assert!(!is_advisor_block("text", None));
        assert!(!is_advisor_block("tool_use", Some("advisor")));
    }

    #[test]
    fn model_predicates_match_opus_and_sonnet_46() {
        assert!(model_supports_advisor("claude-opus-4-6"));
        assert!(model_supports_advisor("CLAUDE-OPUS-4-6")); // case-insensitive
        assert!(model_supports_advisor("claude-sonnet-4-6-20250101"));
        assert!(is_valid_advisor_model("claude-opus-4-6"));
        assert!(is_valid_advisor_model("claude-sonnet-4-6"));
    }

    #[test]
    fn model_predicates_reject_other_models_for_non_ant() {
        // USER_TYPE=ant would flip these; we clear it explicitly to
        // isolate the model-string match.
        let mut env = ScopedEnv::new();
        env.set("USER_TYPE", "external");
        assert!(!model_supports_advisor("claude-haiku-4-5"));
        assert!(!model_supports_advisor("claude-opus-4-5"));
        assert!(!is_valid_advisor_model("claude-haiku-4-5"));
        assert!(!is_valid_advisor_model("gpt-4"));
    }

    #[test]
    fn ant_user_type_accepts_any_model() {
        let mut env = ScopedEnv::new();
        env.set("USER_TYPE", "ant");
        assert!(model_supports_advisor("anything-goes-here"));
        assert!(is_valid_advisor_model("custom-advisor-v2"));
    }

    #[test]
    fn advisor_enabled_requires_growthbook_flag() {
        let mut env = ScopedEnv::new();
        // Force firstParty by unsetting competing provider envs. Then
        // disable experimental betas toggle so the beta-gate is clean.
        env.set("CLAUDE_CODE_USE_BEDROCK", "");
        env.set("CLAUDE_CODE_USE_VERTEX", "");
        env.set("CLAUDE_CODE_USE_FOUNDRY", "");
        env.set("CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS", "");
        env.set("CLAUDE_CODE_DISABLE_ADVISOR_TOOL", "");
        // Config with enabled=None → falsy
        let cfg = AdvisorConfig::default();
        assert!(!is_advisor_enabled(&cfg));
        // enabled=Some(true) → true
        let cfg = AdvisorConfig {
            enabled: Some(true),
            ..Default::default()
        };
        assert!(is_advisor_enabled(&cfg));
    }

    #[test]
    fn advisor_disabled_kill_switch_wins() {
        let mut env = ScopedEnv::new();
        env.set("CLAUDE_CODE_USE_BEDROCK", "");
        env.set("CLAUDE_CODE_USE_VERTEX", "");
        env.set("CLAUDE_CODE_USE_FOUNDRY", "");
        env.set("CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS", "");
        env.set("CLAUDE_CODE_DISABLE_ADVISOR_TOOL", "1");
        let cfg = AdvisorConfig {
            enabled: Some(true),
            ..Default::default()
        };
        assert!(
            !is_advisor_enabled(&cfg),
            "kill switch must override growthbook"
        );
    }

    #[test]
    fn advisor_disabled_on_bedrock() {
        let mut env = ScopedEnv::new();
        env.set("CLAUDE_CODE_USE_BEDROCK", "1");
        env.set("CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS", "");
        env.set("CLAUDE_CODE_DISABLE_ADVISOR_TOOL", "");
        let cfg = AdvisorConfig {
            enabled: Some(true),
            ..Default::default()
        };
        assert!(
            !is_advisor_enabled(&cfg),
            "advisor beta is 1P-only; must stay disabled on Bedrock"
        );
    }

    #[test]
    fn advisor_enabled_on_foundry() {
        let mut env = ScopedEnv::new();
        env.set("CLAUDE_CODE_USE_BEDROCK", "");
        env.set("CLAUDE_CODE_USE_VERTEX", "");
        env.set("CLAUDE_CODE_USE_FOUNDRY", "1");
        env.set("CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS", "");
        env.set("CLAUDE_CODE_DISABLE_ADVISOR_TOOL", "");
        let cfg = AdvisorConfig {
            enabled: Some(true),
            ..Default::default()
        };
        assert!(
            is_advisor_enabled(&cfg),
            "Foundry counts as first-party for the advisor beta"
        );
    }

    #[test]
    fn experiment_advisor_models_needs_both_fields() {
        let mut env = ScopedEnv::new();
        env.set("CLAUDE_CODE_USE_BEDROCK", "");
        env.set("CLAUDE_CODE_USE_VERTEX", "");
        env.set("CLAUDE_CODE_USE_FOUNDRY", "");
        env.set("CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS", "");
        env.set("CLAUDE_CODE_DISABLE_ADVISOR_TOOL", "");
        // Both models present, user-configure off → returns the pair.
        let cfg = AdvisorConfig {
            enabled: Some(true),
            can_user_configure: Some(false),
            base_model: Some("claude-opus-4-6".into()),
            advisor_model: Some("claude-sonnet-4-6".into()),
        };
        assert_eq!(
            get_experiment_advisor_models(&cfg),
            Some(("claude-opus-4-6".into(), "claude-sonnet-4-6".into()))
        );
        // User-configure on → experiment doesn't dictate.
        let cfg = AdvisorConfig {
            can_user_configure: Some(true),
            ..cfg
        };
        assert_eq!(get_experiment_advisor_models(&cfg), None);
        // Missing advisor_model → None.
        let cfg = AdvisorConfig {
            enabled: Some(true),
            can_user_configure: Some(false),
            base_model: Some("claude-opus-4-6".into()),
            advisor_model: None,
        };
        assert_eq!(get_experiment_advisor_models(&cfg), None);
    }

    #[test]
    fn advisor_prompt_is_verbatim() {
        // Spot-check the first and last lines to catch accidental
        // re-wording.
        assert!(ADVISOR_TOOL_INSTRUCTIONS.starts_with("# Advisor Tool\n"));
        assert!(ADVISOR_TOOL_INSTRUCTIONS
            .ends_with("a reconcile call is cheaper than committing to the wrong branch."));
        // Sanity: the prompt keeps its key imperatives.
        assert!(ADVISOR_TOOL_INSTRUCTIONS.contains("Call advisor BEFORE substantive work"));
        assert!(ADVISOR_TOOL_INSTRUCTIONS.contains("When stuck"));
    }
}
