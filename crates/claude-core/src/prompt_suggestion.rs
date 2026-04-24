//! Pure-logic layer for the prompt-suggestion service.
//!
//! Port of TS `services/PromptSuggestion/promptSuggestion.ts`
//! — the **predicate + gate** slice. The async orchestration
//! (`tryGenerateSuggestion`, `runForkedAgent` integration,
//! speculation pipelining) depends on the forked-agent
//!   machinery + REPL hook infrastructure that isn't part of
//!   this crate; those stay in application-layer code.
//!
//! # What's ported here
//!
//! - `PromptVariant` — experimental variant tag.
//! - `PromptSuggestionEnablement` — gate outcome with explicit
//!   reason, replacing TS's bare boolean so callers can log
//!   the specific `source` value that fired.
//! - `evaluate_prompt_suggestion_enablement` — pure,
//!   parameterised gate. Caller injects env var + GrowthBook
//!   value + settings flag + session flags; the function
//!   orders the checks exactly like TS and reports which gate
//!   short-circuited.
//! - `PromptSuggestionSuppressReason` — 5-variant enum
//!   matching TS `getSuggestionSuppressReason` return values.
//! - `evaluate_suggestion_suppression` — consumes a small
//!   data snapshot of the fields the TS function reads off
//!   `AppState`; returns an `Option<PromptSuggestionSuppressReason>`.
//!
//! # What's NOT ported
//!
//! `tryGenerateSuggestion` (async orchestration with
//! `runForkedAgent`, abort controllers, GrowthBook sampling,
//! REPL hook callbacks) — those reach into forked-agent
//! plus speculation and analytics subsystems not in this crate.
//! Callers in the application layer wire those themselves and
//! consult the predicates here for the gating decisions.

use crate::errors_util::{is_env_definitely_falsy, is_env_truthy};
use crate::permissions::types::PermissionMode;
use serde::{Deserialize, Serialize};

/// System prompt sent to the speculation subagent that produces a
/// single next-action suggestion. Shown back in the REPL as a
/// greyed-out prediction of what the user would naturally type
/// next. Verbatim port of TS
/// `services/PromptSuggestion/promptSuggestion.ts:258`
/// `SUGGESTION_PROMPT`.
pub const SUGGESTION_PROMPT: &str =
    "[SUGGESTION MODE: Suggest what the user might naturally type next into Claude Code.]

FIRST: Look at the user's recent messages and original request.

Your job is to predict what THEY would type - not what you think they should do.

THE TEST: Would they think \"I was just about to type that\"?

EXAMPLES:
User asked \"fix the bug and run tests\", bug is fixed → \"run the tests\"
After code written → \"try it out\"
Claude offers options → suggest the one the user would likely pick, based on conversation
Claude asks to continue → \"yes\" or \"go ahead\"
Task complete, obvious follow-up → \"commit this\" or \"push it\"
After error or misunderstanding → silence (let them assess/correct)

Be specific: \"run the tests\" beats \"continue\".

NEVER SUGGEST:
- Evaluative (\"looks good\", \"thanks\")
- Questions (\"what about...?\")
- Claude-voice (\"Let me...\", \"I'll...\", \"Here's...\")
- New ideas they didn't ask about
- Multiple sentences

Stay silent if the next step isn't obvious from what the user said.

Format: 2-12 words, match the user's style. Or nothing.

Reply with ONLY the suggestion, no quotes or explanation.";

/// Experimental variant for suggestion prompt shape. TS
/// `PromptVariant = 'user_intent' | 'stated_intent'`; TS always
/// returns `'user_intent'` from `getPromptVariant()` today (the
/// other arm is reserved for a future A/B).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptVariant {
    UserIntent,
    StatedIntent,
}

impl PromptVariant {
    /// Matches TS `getPromptVariant()` — currently always
    /// returns `UserIntent`. Kept as a function rather than a
    /// constant so the A/B can be wired without churning call
    /// sites.
    pub fn current() -> Self {
        Self::UserIntent
    }
}

/// Source that resolved the final enablement. Mirrors the TS
/// `source:` analytics tag so callers can emit the exact
/// string from
/// `tengu_prompt_suggestion_init`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnablementSource {
    /// `CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION` env override
    /// (either truthy or falsy).
    Env,
    /// GrowthBook `tengu_chomp_inflection` flag.
    Growthbook,
    /// `getIsNonInteractiveSession()` returned true.
    NonInteractive,
    /// Agent-swarm teammate session (only leader shows
    /// suggestions).
    SwarmTeammate,
    /// Explicit `promptSuggestionEnabled` setting.
    Setting,
}

/// Gate outcome. TS returns a bare `boolean` + emits an event;
/// this port surfaces both pieces so the caller can decide
/// what to log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromptSuggestionEnablement {
    pub enabled: bool,
    pub source: EnablementSource,
}

/// Inputs consumed by the gate. Kept as a data struct so
/// callers can build it once from their runtime state + pass
/// to the pure evaluator.
#[derive(Debug, Clone)]
pub struct EnablementInputs<'a> {
    /// Value of `CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION` — the
    /// env override that wins over everything else.
    pub env_override: Option<&'a str>,
    /// GrowthBook flag `tengu_chomp_inflection`. Default
    /// `false` in TS.
    pub growthbook_enabled: bool,
    /// Session is non-interactive (print mode, piped stdin,
    /// SDK from headless).
    pub non_interactive: bool,
    /// Agent-swarm teammate (non-leader). Suppresses
    /// suggestions so only the leader's prompt shows one.
    pub swarm_teammate: bool,
    /// Settings flag `promptSuggestionEnabled`. TS default
    /// `true` when absent — the gate uses `setting != Some(false)`.
    pub setting_enabled: Option<bool>,
}

/// Evaluate the gate. Pure function; no side effects.
///
/// Check order matches TS verbatim:
/// 1. env override truthy → enabled, source=env.
/// 2. env override falsy → disabled, source=env.
/// 3. GrowthBook flag false → disabled, source=growthbook.
/// 4. Non-interactive session → disabled, source=non_interactive.
/// 5. Swarm teammate → disabled, source=swarm_teammate.
/// 6. Default: `setting_enabled != Some(false)`, source=setting.
pub fn evaluate_prompt_suggestion_enablement(
    inputs: &EnablementInputs<'_>,
) -> PromptSuggestionEnablement {
    // Env override wins regardless of anything else. TS reads
    // `process.env.CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION` and
    // checks both falsy + truthy branches separately, returning
    // the env source in both cases.
    if let Some(raw) = inputs.env_override {
        if is_env_definitely_falsy_str(raw) {
            return PromptSuggestionEnablement {
                enabled: false,
                source: EnablementSource::Env,
            };
        }
        if is_env_truthy_str(raw) {
            return PromptSuggestionEnablement {
                enabled: true,
                source: EnablementSource::Env,
            };
        }
    }

    if !inputs.growthbook_enabled {
        return PromptSuggestionEnablement {
            enabled: false,
            source: EnablementSource::Growthbook,
        };
    }

    if inputs.non_interactive {
        return PromptSuggestionEnablement {
            enabled: false,
            source: EnablementSource::NonInteractive,
        };
    }

    if inputs.swarm_teammate {
        return PromptSuggestionEnablement {
            enabled: false,
            source: EnablementSource::SwarmTeammate,
        };
    }

    // TS: `const enabled = getInitialSettings()?.promptSuggestionEnabled !== false`
    // Default-true when the setting is absent; explicit `false`
    // disables. Rust: `setting != Some(false)`.
    let enabled = inputs.setting_enabled != Some(false);
    PromptSuggestionEnablement {
        enabled,
        source: EnablementSource::Setting,
    }
}

/// Thin adapter that consults real env vars — for callers
/// that want the common case without building
/// [`EnablementInputs`] manually. Keeps the pure evaluator
/// separate from env access for testability.
pub fn evaluate_prompt_suggestion_enablement_from_env(
    env_var_name: &str,
    growthbook_enabled: bool,
    non_interactive: bool,
    swarm_teammate: bool,
    setting_enabled: Option<bool>,
) -> PromptSuggestionEnablement {
    let raw = std::env::var(env_var_name).ok();
    evaluate_prompt_suggestion_enablement(&EnablementInputs {
        env_override: raw.as_deref(),
        growthbook_enabled,
        non_interactive,
        swarm_teammate,
        setting_enabled,
    })
}

/// Why suggestions are suppressed, if they are. Matches the
/// 5 return values of TS `getSuggestionSuppressReason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptSuggestionSuppressReason {
    Disabled,
    PendingPermission,
    ElicitationActive,
    PlanMode,
    RateLimit,
}

/// Data snapshot consumed by [`evaluate_suggestion_suppression`].
/// Represents the subset of `AppState` the TS function reads.
/// Callers build it from whatever live state they hold.
#[derive(Debug, Clone)]
pub struct SuppressionInputs {
    pub prompt_suggestion_enabled: bool,
    pub pending_worker_request: bool,
    pub pending_sandbox_request: bool,
    pub elicitation_queue_nonempty: bool,
    pub permission_mode: PermissionMode,
    /// External-user rate-limit status. `true` means
    /// suggestions are currently rate-limited. Matches TS
    /// `currentLimits.status !== 'allowed'` for external
    /// users.
    pub rate_limited: bool,
    /// TS check is gated on `process.env.USER_TYPE === 'external'`
    /// — let the caller pre-compute that conjunction.
    pub is_external_user: bool,
}

/// Returns `Some(reason)` when suggestions should NOT be
/// generated, `None` when generation is allowed. Matches TS
/// `getSuggestionSuppressReason` check order exactly.
pub fn evaluate_suggestion_suppression(
    inputs: &SuppressionInputs,
) -> Option<PromptSuggestionSuppressReason> {
    if !inputs.prompt_suggestion_enabled {
        return Some(PromptSuggestionSuppressReason::Disabled);
    }
    if inputs.pending_worker_request || inputs.pending_sandbox_request {
        return Some(PromptSuggestionSuppressReason::PendingPermission);
    }
    if inputs.elicitation_queue_nonempty {
        return Some(PromptSuggestionSuppressReason::ElicitationActive);
    }
    if inputs.permission_mode == PermissionMode::Plan {
        return Some(PromptSuggestionSuppressReason::PlanMode);
    }
    if inputs.is_external_user && inputs.rate_limited {
        return Some(PromptSuggestionSuppressReason::RateLimit);
    }
    None
}

// Helpers — the env-reading forms of the truthy/falsy checks
// take an `&str` name and read the env. We already have values
// in hand, so these string-level variants call through.
fn is_env_definitely_falsy_str(value: &str) -> bool {
    // Reuse the existing `errors_util::is_env_definitely_falsy`
    // semantic by setting an isolated env var. Simpler: inline
    // the recognised falsy strings — matches the library's
    // allow-list.
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "0" | "false" | "no" | "off"
    )
}

fn is_env_truthy_str(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

// Silence unused-import warnings in scope where only the
// string-helpers are touched.
#[allow(dead_code)]
fn _unused_imports_hint(name: &str) -> (bool, bool) {
    (is_env_definitely_falsy(name), is_env_truthy(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_inputs<'a>() -> EnablementInputs<'a> {
        EnablementInputs {
            env_override: None,
            growthbook_enabled: true,
            non_interactive: false,
            swarm_teammate: false,
            setting_enabled: None,
        }
    }

    #[test]
    fn variant_current_is_user_intent() {
        assert_eq!(PromptVariant::current(), PromptVariant::UserIntent);
    }

    #[test]
    fn env_override_truthy_wins() {
        let inputs = EnablementInputs {
            env_override: Some("1"),
            growthbook_enabled: false, // would otherwise disable
            ..default_inputs()
        };
        let out = evaluate_prompt_suggestion_enablement(&inputs);
        assert!(out.enabled);
        assert_eq!(out.source, EnablementSource::Env);
    }

    #[test]
    fn env_override_falsy_wins() {
        let inputs = EnablementInputs {
            env_override: Some("0"),
            growthbook_enabled: true,
            setting_enabled: Some(true),
            ..default_inputs()
        };
        let out = evaluate_prompt_suggestion_enablement(&inputs);
        assert!(!out.enabled);
        assert_eq!(out.source, EnablementSource::Env);
    }

    #[test]
    fn env_override_unrecognised_value_falls_through() {
        // A random non-truthy/non-falsy string doesn't gate.
        let inputs = EnablementInputs {
            env_override: Some("maybe"),
            ..default_inputs()
        };
        let out = evaluate_prompt_suggestion_enablement(&inputs);
        // Setting default is None → enabled, source=setting.
        assert!(out.enabled);
        assert_eq!(out.source, EnablementSource::Setting);
    }

    #[test]
    fn growthbook_disabled_fires_after_env() {
        let inputs = EnablementInputs {
            growthbook_enabled: false,
            ..default_inputs()
        };
        let out = evaluate_prompt_suggestion_enablement(&inputs);
        assert!(!out.enabled);
        assert_eq!(out.source, EnablementSource::Growthbook);
    }

    #[test]
    fn non_interactive_disables() {
        let inputs = EnablementInputs {
            non_interactive: true,
            ..default_inputs()
        };
        let out = evaluate_prompt_suggestion_enablement(&inputs);
        assert!(!out.enabled);
        assert_eq!(out.source, EnablementSource::NonInteractive);
    }

    #[test]
    fn swarm_teammate_disables() {
        let inputs = EnablementInputs {
            swarm_teammate: true,
            ..default_inputs()
        };
        let out = evaluate_prompt_suggestion_enablement(&inputs);
        assert!(!out.enabled);
        assert_eq!(out.source, EnablementSource::SwarmTeammate);
    }

    #[test]
    fn setting_false_disables_and_source_is_setting() {
        let inputs = EnablementInputs {
            setting_enabled: Some(false),
            ..default_inputs()
        };
        let out = evaluate_prompt_suggestion_enablement(&inputs);
        assert!(!out.enabled);
        assert_eq!(out.source, EnablementSource::Setting);
    }

    #[test]
    fn setting_missing_defaults_to_enabled() {
        let inputs = default_inputs();
        let out = evaluate_prompt_suggestion_enablement(&inputs);
        assert!(out.enabled);
        assert_eq!(out.source, EnablementSource::Setting);
    }

    #[test]
    fn setting_true_explicit_enabled() {
        let inputs = EnablementInputs {
            setting_enabled: Some(true),
            ..default_inputs()
        };
        let out = evaluate_prompt_suggestion_enablement(&inputs);
        assert!(out.enabled);
    }

    #[test]
    fn env_adapter_respects_real_env() {
        // Store + restore around the adapter call.
        let key = "CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION_TEST_ONLY";
        std::env::set_var(key, "0");
        let out = evaluate_prompt_suggestion_enablement_from_env(key, true, false, false, None);
        assert!(!out.enabled);
        assert_eq!(out.source, EnablementSource::Env);
        std::env::remove_var(key);
    }

    // --- suppression -----------------------------------------

    fn default_suppression() -> SuppressionInputs {
        SuppressionInputs {
            prompt_suggestion_enabled: true,
            pending_worker_request: false,
            pending_sandbox_request: false,
            elicitation_queue_nonempty: false,
            permission_mode: PermissionMode::Default,
            rate_limited: false,
            is_external_user: false,
        }
    }

    #[test]
    fn no_suppression_when_all_clear() {
        assert!(evaluate_suggestion_suppression(&default_suppression()).is_none());
    }

    #[test]
    fn disabled_flag_suppresses() {
        let mut s = default_suppression();
        s.prompt_suggestion_enabled = false;
        assert_eq!(
            evaluate_suggestion_suppression(&s),
            Some(PromptSuggestionSuppressReason::Disabled)
        );
    }

    #[test]
    fn pending_worker_request_suppresses() {
        let mut s = default_suppression();
        s.pending_worker_request = true;
        assert_eq!(
            evaluate_suggestion_suppression(&s),
            Some(PromptSuggestionSuppressReason::PendingPermission)
        );
    }

    #[test]
    fn pending_sandbox_request_suppresses() {
        let mut s = default_suppression();
        s.pending_sandbox_request = true;
        assert_eq!(
            evaluate_suggestion_suppression(&s),
            Some(PromptSuggestionSuppressReason::PendingPermission)
        );
    }

    #[test]
    fn elicitation_queue_suppresses() {
        let mut s = default_suppression();
        s.elicitation_queue_nonempty = true;
        assert_eq!(
            evaluate_suggestion_suppression(&s),
            Some(PromptSuggestionSuppressReason::ElicitationActive)
        );
    }

    #[test]
    fn plan_mode_suppresses() {
        let mut s = default_suppression();
        s.permission_mode = PermissionMode::Plan;
        assert_eq!(
            evaluate_suggestion_suppression(&s),
            Some(PromptSuggestionSuppressReason::PlanMode)
        );
    }

    #[test]
    fn rate_limit_only_fires_for_external_users() {
        let mut s = default_suppression();
        s.rate_limited = true;
        s.is_external_user = false; // ant / internal user
        assert!(evaluate_suggestion_suppression(&s).is_none());
        s.is_external_user = true;
        assert_eq!(
            evaluate_suggestion_suppression(&s),
            Some(PromptSuggestionSuppressReason::RateLimit)
        );
    }

    #[test]
    fn check_order_matches_ts() {
        // TS checks in this order: disabled → pending → elicitation →
        // plan → rate_limit. Simulating all-true except the first
        // that fires should yield the earliest-match reason.
        let mut s = default_suppression();
        s.prompt_suggestion_enabled = false;
        s.pending_worker_request = true;
        s.elicitation_queue_nonempty = true;
        s.permission_mode = PermissionMode::Plan;
        s.is_external_user = true;
        s.rate_limited = true;
        // `disabled` is first.
        assert_eq!(
            evaluate_suggestion_suppression(&s),
            Some(PromptSuggestionSuppressReason::Disabled)
        );
    }

    #[test]
    fn variant_serialises_snake_case() {
        let v = PromptVariant::UserIntent;
        assert_eq!(
            serde_json::to_value(v).unwrap(),
            serde_json::json!("user_intent")
        );
    }

    #[test]
    fn enablement_source_wire_format() {
        assert_eq!(
            serde_json::to_value(EnablementSource::NonInteractive).unwrap(),
            serde_json::json!("non_interactive")
        );
        assert_eq!(
            serde_json::to_value(EnablementSource::SwarmTeammate).unwrap(),
            serde_json::json!("swarm_teammate")
        );
    }

    // --- suggestion prompt text ------------------------------

    #[test]
    fn suggestion_prompt_has_expected_anchors() {
        assert!(SUGGESTION_PROMPT.starts_with("[SUGGESTION MODE:"));
        assert!(SUGGESTION_PROMPT.contains("THE TEST:"));
        assert!(SUGGESTION_PROMPT.contains("NEVER SUGGEST:"));
        assert!(SUGGESTION_PROMPT.contains("Format: 2-12 words"));
        // Closing instruction must stay — it's the only line that
        // guarantees the subagent doesn't wrap the suggestion in
        // extra commentary.
        assert!(SUGGESTION_PROMPT.ends_with("no quotes or explanation."));
    }

    #[test]
    fn suggestion_prompt_preserves_ts_examples() {
        assert!(SUGGESTION_PROMPT.contains("\"fix the bug and run tests\""));
        assert!(SUGGESTION_PROMPT.contains("\"run the tests\""));
        assert!(SUGGESTION_PROMPT.contains("\"commit this\""));
    }
}
