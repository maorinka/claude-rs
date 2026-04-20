//! `ToolPermissionContext` — aggregate of permission state
//! consulted on every tool invocation.
//!
//! Port of TS `Tool.ts:122-138` (`ToolPermissionContext =
//! DeepImmutable<{...}>`) plus the nested types from
//! `types/permissions.ts` (`AdditionalWorkingDirectory`,
//! `WorkingDirectorySource`, `ToolPermissionRulesBySource`).
//!
//! Replaces the `Value` placeholder on `AppState.
//! tool_permission_context` — Codex CR step-2 verdict: "It is
//! acceptable for step 3 if the host only stores/forwards it
//! and no Rust code needs typed reads over its internals yet.
//! It becomes a blocker the moment permission decisions or UI
//! derivation in Rust need structural access."
//!
//! Landing the typed shape now means future permission-
//! decision code in Rust can rely on typed reads without
//! chasing the `Value` everywhere.
//!
//! # Relationship to existing permissions module
//!
//! `permissions::types::PermissionMode` + `PermissionRuleSource`
//! are already ported — imported here, not re-defined. This
//! module is the AGGREGATE that tool invocations consume,
//! not a re-port.

use crate::permissions::types::{PermissionMode, PermissionRuleSource};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Where an additional working directory came from. TS
/// `WorkingDirectorySource = PermissionRuleSource` — a bare
/// type alias. Reusing the existing enum directly.
pub type WorkingDirectorySource = PermissionRuleSource;

/// One entry in `ToolPermissionContext.additional_working_directories`.
/// TS `types/permissions.ts:143-146`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdditionalWorkingDirectory {
    pub path: String,
    pub source: WorkingDirectorySource,
}

/// Map from rule source to the rule string list for that
/// source. TS `ToolPermissionRulesBySource = { [T in
/// PermissionRuleSource]?: string[] }`.
///
/// Stored as `HashMap<PermissionRuleSource, Vec<String>>` —
/// the TS `[T in ...]` partial-mapped shape is idiomatic to
/// model as a map in Rust.
pub type ToolPermissionRulesBySource = HashMap<PermissionRuleSource, Vec<String>>;

/// Permission context consumed by every tool. TS
/// `Tool.ts:122-138`. `DeepImmutable` is a TS construct; in
/// Rust, immutability is enforced by borrowing — callers pass
/// `&ToolPermissionContext` to `canUseTool`-shaped checks and
/// the compiler guarantees no mutation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolPermissionContext {
    pub mode: PermissionMode,

    /// TS uses `ReadonlyMap<string, AdditionalWorkingDirectory>`
    /// keyed on the absolute path. Rust: `HashMap<String, ...>`.
    #[serde(default)]
    pub additional_working_directories: HashMap<String, AdditionalWorkingDirectory>,

    #[serde(default, rename = "alwaysAllowRules")]
    pub always_allow_rules: ToolPermissionRulesBySource,

    #[serde(default, rename = "alwaysDenyRules")]
    pub always_deny_rules: ToolPermissionRulesBySource,

    #[serde(default, rename = "alwaysAskRules")]
    pub always_ask_rules: ToolPermissionRulesBySource,

    #[serde(default, rename = "isBypassPermissionsModeAvailable")]
    pub is_bypass_permissions_mode_available: bool,

    #[serde(default, rename = "isAutoModeAvailable", skip_serializing_if = "Option::is_none")]
    pub is_auto_mode_available: Option<bool>,

    /// Rules stripped as "dangerous" per policy. TS comment:
    /// only present when a rule was stripped.
    #[serde(
        default,
        rename = "strippedDangerousRules",
        skip_serializing_if = "Option::is_none"
    )]
    pub stripped_dangerous_rules: Option<ToolPermissionRulesBySource>,

    /// Background agents without a UI surface auto-deny
    /// prompts.
    #[serde(
        default,
        rename = "shouldAvoidPermissionPrompts",
        skip_serializing_if = "Option::is_none"
    )]
    pub should_avoid_permission_prompts: Option<bool>,

    /// Coordinator workers wait for automated checks
    /// (classifier, hooks) before showing a permission
    /// dialog.
    #[serde(
        default,
        rename = "awaitAutomatedChecksBeforeDialog",
        skip_serializing_if = "Option::is_none"
    )]
    pub await_automated_checks_before_dialog: Option<bool>,

    /// Prior permission mode captured before model-initiated
    /// plan-mode entry — used to restore on exit.
    #[serde(
        default,
        rename = "prePlanMode",
        skip_serializing_if = "Option::is_none"
    )]
    pub pre_plan_mode: Option<PermissionMode>,
}

impl ToolPermissionContext {
    /// Empty context with `Default` mode. Matches TS
    /// `getEmptyToolPermissionContext()` at
    /// `Tool.ts:140-148`.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Convenience predicate. Returns whether any source
    /// entry exists for the given rule-string in
    /// `always_allow_rules`.
    pub fn is_always_allowed(&self, rule: &str) -> bool {
        self.always_allow_rules
            .values()
            .any(|rules| rules.iter().any(|r| r == rule))
    }

    /// Same predicate for `always_deny_rules`.
    pub fn is_always_denied(&self, rule: &str) -> bool {
        self.always_deny_rules
            .values()
            .any(|rules| rules.iter().any(|r| r == rule))
    }

    /// Same for `always_ask_rules`.
    pub fn is_always_ask(&self, rule: &str) -> bool {
        self.always_ask_rules
            .values()
            .any(|rules| rules.iter().any(|r| r == rule))
    }

    /// Record an additional working directory. Matches the
    /// TS `setAppState(prev => { additionalWorkingDirectories:
    /// new Map([...prev, [path, entry]]) })` pattern.
    pub fn add_working_directory(&mut self, path: String, source: WorkingDirectorySource) {
        self.additional_working_directories.insert(
            path.clone(),
            AdditionalWorkingDirectory { path, source },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn empty_context_has_default_mode() {
        let ctx = ToolPermissionContext::empty();
        assert_eq!(ctx.mode, PermissionMode::Default);
        assert!(ctx.additional_working_directories.is_empty());
        assert!(!ctx.is_bypass_permissions_mode_available);
    }

    #[test]
    fn allow_rule_lookup_works_across_sources() {
        let mut ctx = ToolPermissionContext::empty();
        ctx.always_allow_rules.insert(
            PermissionRuleSource::UserSettings,
            vec!["Bash(ls:*)".into()],
        );
        ctx.always_allow_rules.insert(
            PermissionRuleSource::ProjectSettings,
            vec!["Read(/etc/hosts)".into()],
        );
        assert!(ctx.is_always_allowed("Bash(ls:*)"));
        assert!(ctx.is_always_allowed("Read(/etc/hosts)"));
        assert!(!ctx.is_always_allowed("Missing"));
    }

    #[test]
    fn deny_and_ask_lookups() {
        let mut ctx = ToolPermissionContext::empty();
        ctx.always_deny_rules.insert(
            PermissionRuleSource::PolicySettings,
            vec!["Bash(rm:*)".into()],
        );
        ctx.always_ask_rules.insert(
            PermissionRuleSource::Session,
            vec!["WebFetch(*)".into()],
        );
        assert!(ctx.is_always_denied("Bash(rm:*)"));
        assert!(ctx.is_always_ask("WebFetch(*)"));
        assert!(!ctx.is_always_ask("Something else"));
    }

    #[test]
    fn add_working_directory_indexes_by_path() {
        let mut ctx = ToolPermissionContext::empty();
        ctx.add_working_directory(
            "/src/override".into(),
            PermissionRuleSource::CliArg,
        );
        assert_eq!(ctx.additional_working_directories.len(), 1);
        let entry = &ctx.additional_working_directories["/src/override"];
        assert_eq!(entry.path, "/src/override");
        assert_eq!(entry.source, PermissionRuleSource::CliArg);
    }

    #[test]
    fn add_working_directory_overwrites_same_path() {
        let mut ctx = ToolPermissionContext::empty();
        ctx.add_working_directory("/x".into(), PermissionRuleSource::CliArg);
        ctx.add_working_directory("/x".into(), PermissionRuleSource::UserSettings);
        // Latest write wins, matching JS Map `.set`.
        assert_eq!(
            ctx.additional_working_directories["/x"].source,
            PermissionRuleSource::UserSettings
        );
        assert_eq!(ctx.additional_working_directories.len(), 1);
    }

    #[test]
    fn serialises_camel_case_for_wire_parity() {
        let mut ctx = ToolPermissionContext::empty();
        ctx.is_bypass_permissions_mode_available = true;
        ctx.is_auto_mode_available = Some(true);
        ctx.should_avoid_permission_prompts = Some(false);
        ctx.await_automated_checks_before_dialog = Some(true);
        ctx.pre_plan_mode = Some(PermissionMode::Default);
        let v = serde_json::to_value(&ctx).unwrap();
        // Pin the camelCase keys that cross the wire.
        for k in [
            "alwaysAllowRules",
            "alwaysDenyRules",
            "alwaysAskRules",
            "isBypassPermissionsModeAvailable",
            "isAutoModeAvailable",
            "shouldAvoidPermissionPrompts",
            "awaitAutomatedChecksBeforeDialog",
            "prePlanMode",
        ] {
            assert!(v.get(k).is_some(), "missing key: {k}");
        }
    }

    #[test]
    fn optional_fields_omitted_when_none() {
        let ctx = ToolPermissionContext::empty();
        let v = serde_json::to_value(&ctx).unwrap();
        let obj = v.as_object().unwrap();
        assert!(obj.get("isAutoModeAvailable").is_none());
        assert!(obj.get("strippedDangerousRules").is_none());
        assert!(obj.get("shouldAvoidPermissionPrompts").is_none());
        assert!(obj.get("awaitAutomatedChecksBeforeDialog").is_none());
        assert!(obj.get("prePlanMode").is_none());
    }

    #[test]
    fn deserialises_ts_wire_shape() {
        let wire = json!({
            "mode": "acceptEdits",
            "additional_working_directories": {
                "/override": {
                    "path": "/override",
                    "source": "cliArg"
                }
            },
            "alwaysAllowRules": {
                "userSettings": ["Bash(ls:*)"]
            },
            "alwaysDenyRules": {},
            "alwaysAskRules": {},
            "isBypassPermissionsModeAvailable": true,
            "isAutoModeAvailable": true
        });
        let ctx: ToolPermissionContext = serde_json::from_value(wire).unwrap();
        assert_eq!(ctx.mode, PermissionMode::AcceptEdits);
        assert_eq!(ctx.additional_working_directories.len(), 1);
        assert!(ctx.is_always_allowed("Bash(ls:*)"));
        assert!(ctx.is_bypass_permissions_mode_available);
        assert_eq!(ctx.is_auto_mode_available, Some(true));
    }

    #[test]
    fn clone_is_cheap_and_equivalent() {
        let mut ctx = ToolPermissionContext::empty();
        ctx.add_working_directory("/x".into(), PermissionRuleSource::CliArg);
        let c = ctx.clone();
        assert_eq!(ctx.additional_working_directories.len(), c.additional_working_directories.len());
    }
}
