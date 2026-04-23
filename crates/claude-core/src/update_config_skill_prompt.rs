//! `/update-config` bundled-skill prompt constants.
//!
//! Port of TS `src/skills/bundled/updateConfig.ts`. Guides the
//! model through modifying `settings.json` files: permissions,
//! env vars, hooks, plugins, MCP servers. The Rust port ships
//! the three reusable text blocks the TS skill assembles plus
//! the [`update_config_prompt`] + [`hooks_only_prompt`] builders
//! that match the skill's two invocation modes. Full skill
//! registration is deferred until the AskUserQuestion / bundled-
//! skill wiring covers it.
//!
//! TS literally interpolates the three sub-blocks into
//! `UPDATE_CONFIG_PROMPT` via `${SETTINGS_EXAMPLES_DOCS}` /
//! `${HOOKS_DOCS}` / `${HOOK_VERIFICATION_FLOW}`. The Rust port
//! keeps them as separate consts so callers (and future audits)
//! can reach for them individually.
//!
//! # Dynamic settings schema
//!
//! TS calls `toJSONSchema(SettingsSchema(), …)` at invocation
//! time and appends a "Full Settings JSON Schema" section. The
//! Rust port leaves that slot as a caller-provided argument —
//! the schema comes from the settings type system, which lives
//! outside this module.

/// The settings-file-locations table + schema reference block.
/// Port of TS `SETTINGS_EXAMPLES_DOCS` in updateConfig.ts:15-104.
pub const UPDATE_CONFIG_SETTINGS_EXAMPLES_DOCS: &str =
    include_str!("prompts/update_config/settings_examples.md");

/// Hooks-configuration reference (structure, events, types,
/// stdin/stdout JSON, common patterns). Port of TS `HOOKS_DOCS`
/// in updateConfig.ts:110-267.
pub const UPDATE_CONFIG_HOOKS_DOCS: &str =
    include_str!("prompts/update_config/hooks_docs.md");

/// Seven-step hook construction + verification flow (dedup,
/// pipe-test, validate, fire-proof, handoff). Port of TS
/// `HOOK_VERIFICATION_FLOW` in updateConfig.ts:269-305.
pub const UPDATE_CONFIG_HOOK_VERIFICATION_FLOW: &str =
    include_str!("prompts/update_config/hook_verification_flow.md");

/// Skeleton of `UPDATE_CONFIG_PROMPT` with three `{{SLOT}}`
/// placeholders that [`update_config_prompt`] fills with the
/// three blocks above. Port of TS `UPDATE_CONFIG_PROMPT` in
/// updateConfig.ts:307-443.
const UPDATE_CONFIG_PROMPT_TEMPLATE: &str =
    include_str!("prompts/update_config/main_prompt.md");

/// Build the full `/update-config` prompt, optionally appending a
/// JSON-schema block + user-request section. Port of TS
/// `getPromptForCommand(args)` at updateConfig.ts:452-472 for the
/// default (non-`[hooks-only]`) path.
///
/// - `settings_json_schema` — pre-stringified JSON schema. When
///   `None`, the `## Full Settings JSON Schema` section is
///   omitted (TS always emits it; the Rust port lets the caller
///   decide since schema generation lives in the settings crate).
/// - `args` — user request; empty ⇒ no `## User Request` block.
pub fn update_config_prompt(settings_json_schema: Option<&str>, args: &str) -> String {
    let base = UPDATE_CONFIG_PROMPT_TEMPLATE
        .replace(
            "{{SETTINGS_EXAMPLES_DOCS}}",
            UPDATE_CONFIG_SETTINGS_EXAMPLES_DOCS,
        )
        .replace("{{HOOKS_DOCS}}", UPDATE_CONFIG_HOOKS_DOCS)
        .replace(
            "{{HOOK_VERIFICATION_FLOW}}",
            UPDATE_CONFIG_HOOK_VERIFICATION_FLOW,
        );

    let mut out = base;
    if let Some(schema) = settings_json_schema {
        out.push_str("\n\n## Full Settings JSON Schema\n\n```json\n");
        out.push_str(schema);
        out.push_str("\n```");
    }
    if !args.is_empty() {
        out.push_str("\n\n## User Request\n\n");
        out.push_str(args);
    }
    out
}

/// Build the `[hooks-only]` variant — the model is handed the
/// hooks reference + verification flow and asked to do a
/// hook-specific task without the full config surface. Port of
/// TS `getPromptForCommand(args)` branch at
/// updateConfig.ts:453-460.
///
/// `task` is the instruction text that follows the `[hooks-only]`
/// prefix. Empty strings omit the `## Task` section.
pub fn hooks_only_prompt(task: &str) -> String {
    let mut out = format!(
        "{UPDATE_CONFIG_HOOKS_DOCS}\n\n{UPDATE_CONFIG_HOOK_VERIFICATION_FLOW}"
    );
    if !task.is_empty() {
        out.push_str("\n\n## Task\n\n");
        out.push_str(task);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_three_sub_blocks_have_canonical_headers() {
        assert!(UPDATE_CONFIG_SETTINGS_EXAMPLES_DOCS.starts_with("## Settings File Locations"));
        assert!(UPDATE_CONFIG_HOOKS_DOCS.starts_with("## Hooks Configuration"));
        assert!(UPDATE_CONFIG_HOOK_VERIFICATION_FLOW
            .starts_with("## Constructing a Hook (with verification)"));
    }

    #[test]
    fn main_prompt_stitches_the_three_blocks() {
        let p = update_config_prompt(None, "");
        assert!(p.starts_with("# Update Config Skill"));
        // Each block's header appears once post-substitution.
        assert!(p.contains("## Settings File Locations"));
        assert!(p.contains("## Hooks Configuration"));
        assert!(p.contains("## Constructing a Hook (with verification)"));
        // No unfilled slots.
        assert!(!p.contains("{{SETTINGS_EXAMPLES_DOCS}}"));
        assert!(!p.contains("{{HOOKS_DOCS}}"));
        assert!(!p.contains("{{HOOK_VERIFICATION_FLOW}}"));
    }

    #[test]
    fn main_prompt_appends_schema_when_provided() {
        let p = update_config_prompt(Some("{ \"type\": \"object\" }"), "");
        assert!(p.contains("## Full Settings JSON Schema"));
        assert!(p.contains("{ \"type\": \"object\" }"));
    }

    #[test]
    fn main_prompt_omits_schema_section_when_none() {
        let p = update_config_prompt(None, "");
        assert!(!p.contains("## Full Settings JSON Schema"));
    }

    #[test]
    fn main_prompt_appends_user_request_when_args_present() {
        let p = update_config_prompt(None, "allow npm commands");
        assert!(p.contains("## User Request\n\nallow npm commands"));
    }

    #[test]
    fn main_prompt_omits_user_request_when_args_empty() {
        let p = update_config_prompt(None, "");
        assert!(!p.contains("## User Request"));
    }

    #[test]
    fn hooks_only_prompt_excludes_settings_examples() {
        let p = hooks_only_prompt("");
        assert!(p.contains("## Hooks Configuration"));
        assert!(p.contains("## Constructing a Hook (with verification)"));
        // `[hooks-only]` mode suppresses the broader settings docs.
        assert!(!p.contains("## Settings File Locations"));
        assert!(!p.contains("## Settings Schema Reference"));
    }

    #[test]
    fn hooks_only_prompt_appends_task_when_provided() {
        let p = hooks_only_prompt("log every bash command");
        assert!(p.contains("## Task\n\nlog every bash command"));
    }

    #[test]
    fn hooks_only_prompt_omits_task_section_when_empty() {
        let p = hooks_only_prompt("");
        assert!(!p.contains("## Task"));
    }

    #[test]
    fn hook_verification_flow_carries_all_seven_steps() {
        let f = UPDATE_CONFIG_HOOK_VERIFICATION_FLOW;
        assert!(f.contains("1. **Dedup check.**"));
        assert!(f.contains("2. **Construct the command for THIS project"));
        assert!(f.contains("3. **Pipe-test the raw command.**"));
        assert!(f.contains("4. **Write the JSON.**"));
        assert!(f.contains("5. **Validate syntax + schema in one shot:**"));
        assert!(f.contains("6. **Prove the hook fires**"));
        assert!(f.contains("7. **Handoff.**"));
    }

    #[test]
    fn hooks_docs_lists_every_event() {
        let d = UPDATE_CONFIG_HOOKS_DOCS;
        for event in &[
            "PermissionRequest",
            "PreToolUse",
            "PostToolUse",
            "PostToolUseFailure",
            "Notification",
            "Stop",
            "PreCompact",
            "PostCompact",
            "UserPromptSubmit",
            "SessionStart",
        ] {
            assert!(
                d.contains(event),
                "hooks_docs missing event `{event}`"
            );
        }
    }
}
