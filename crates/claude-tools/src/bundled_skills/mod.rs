//! Bundled skills — prompt text + registration for the static skills
//! shipped with the CLI.
//!
//! Ports TS `src/skills/bundled/*.ts`. Each bundled skill has a
//! name + description + prompt text + optional isEnabled gate.
//! The Rust port stores the prompt text in per-skill `.md` files
//! loaded via `include_str!` so the bytes live in the binary
//! unchanged (byte-stable for prompt-cache) and diff cleanly
//! against the TS originals.
//!
//! ## Call site
//!
//! Call [`register_bundled_skills`] once at startup (typically
//! from `claude-cli/main.rs` right after `build_default_registry`).
//! Each registrar below is gated on its TS-equivalent condition:
//! - `simplify` — unconditional (all user types)
//! - `stuck`    — ant-only (`USER_TYPE === 'ant'`)
//! - `remember` — ant-only + `auto_memory_enabled()`
//! - `loop`     — `AGENT_TRIGGERS` env flag (matches TS
//!   `isKairosCronEnabled`'s `feature('AGENT_TRIGGERS')` gate)
//!
//! ## TS parity notes
//!
//! TS skills can:
//! 1. Return extra content (`files: SKILL_FILES`) alongside the
//!    main prompt — skipped for this initial batch (none of the
//!    three skills here use it).
//! 2. Append user-provided `args` to the prompt under a
//!    skill-specific header — `simplify` uses
//!    `## Additional Focus`, `stuck` uses `## User-provided
//!    context`, `remember` uses `## Additional context from
//!    user`. Each register call here passes that header via
//!    [`register_skill_with_arg_header`] so the SkillTool emits
//!    matching text at invoke time (instead of the generic
//!    `Arguments: {args}` line).
//! 3. Gate on feature flags (`isAutoMemoryEnabled`,
//!    `isKairosCronEnabled`, etc.). `remember` ports the
//!    `auto_memory_enabled()` gate so the skill is hidden when
//!    auto-memory is off — matches TS `remember.ts:71`.

use crate::cron_tool::is_kairos_cron_enabled;
use crate::skill_tool::{register_skill_full, register_skill_with_arg_header};
use claude_core::memdir::auto_memory_enabled;
use claude_core::user_type;

const SIMPLIFY_PROMPT: &str = include_str!("simplify.md");
const STUCK_PROMPT: &str = include_str!("stuck.md");
const REMEMBER_PROMPT: &str = include_str!("remember.md");
const LOOP_PROMPT: &str = include_str!("loop.md");
const LOOP_USAGE: &str = include_str!("loop_usage.md");

/// Register every bundled skill whose gate passes for the current
/// user type. Idempotent: `register_skill` replaces by name, so
/// calling this twice is harmless.
pub fn register_bundled_skills() {
    register_simplify_skill();
    register_stuck_skill();
    register_remember_skill();
    register_loop_skill();
}

/// Port of TS `registerSimplifySkill`. Reviews changed files for
/// reuse / quality / efficiency and fixes issues found. Launches
/// three review sub-agents in parallel.
pub fn register_simplify_skill() {
    register_skill_with_arg_header(
        "simplify",
        "Review changed code for reuse, quality, and efficiency, then fix any issues found.",
        SIMPLIFY_PROMPT,
        Some("Additional Focus"),
    );
}

/// Port of TS `registerStuckSkill`. Ant-only diagnostic that
/// scans local `claude`/`cli` processes for stuck/slow sessions
/// and posts a report to #claude-code-feedback.
pub fn register_stuck_skill() {
    if !user_type::is_ant() {
        return;
    }
    register_skill_with_arg_header(
        "stuck",
        "[ANT-ONLY] Investigate frozen/stuck/slow Claude Code sessions on this machine and post a diagnostic report to #claude-code-feedback.",
        STUCK_PROMPT,
        Some("User-provided context"),
    );
}

/// Port of TS `registerRememberSkill`. Ant-only memory-review
/// skill that classifies auto-memory entries across CLAUDE.md,
/// CLAUDE.local.md, team memory, and auto-memory layers.
///
/// Double-gated (both gates match TS `remember.ts:5 + :71`):
/// - `USER_TYPE === 'ant'`
/// - `auto_memory_enabled()` — the skill's instructions assume
///   "your auto-memory content is already in your system prompt";
///   hiding when auto-memory is off prevents a
///   discoverable-but-nonfunctional entry in the registry.
pub fn register_remember_skill() {
    if !user_type::is_ant() {
        return;
    }
    if !auto_memory_enabled() {
        return;
    }
    register_skill_with_arg_header(
        "remember",
        "Review auto-memory entries and propose promotions to CLAUDE.md, CLAUDE.local.md, or shared memory. Also detects outdated, conflicting, and duplicate entries across memory layers.",
        REMEMBER_PROMPT,
        Some("Additional context from user"),
    );
}

/// Port of TS `registerLoopSkill`. Runs a prompt or slash command
/// on a recurring interval via CronCreate. Empty-args short-
/// circuits to a usage message; non-empty args get appended under
/// the `## Input` header so the model parses them as in TS
/// `buildPrompt(args)`.
///
/// Gate: `isKairosCronEnabled` in TS requires the
/// `feature('AGENT_TRIGGERS')` build flag + a GrowthBook flag.
/// Rust has the same AGENT_TRIGGERS env gate on the cron tool
/// registry (see `claude-tools/src/lib.rs`). We reuse it here so
/// `/loop` and the `CronCreate` tool it references show up
/// together — registering `/loop` without the cron tools would
/// give the model a broken reference.
///
/// TS interpolations baked into loop.md as literals:
/// - `${CRON_CREATE_TOOL_NAME}` → `CronCreate`
/// - `${CRON_DELETE_TOOL_NAME}` → `CronDelete`
/// - `${DEFAULT_INTERVAL}` → `10m`
/// - `${DEFAULT_MAX_AGE_DAYS}` → `30`
pub fn register_loop_skill() {
    if !is_kairos_cron_enabled() {
        return;
    }
    register_skill_full(
        "loop",
        "Run a prompt or slash command on a recurring interval (e.g. /loop 5m /foo, defaults to 10m)",
        LOOP_PROMPT,
        Some("Input"),
        Some(LOOP_USAGE),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skill_tool::{clear_skills, list_skills};
    use std::sync::Mutex;

    // Tests mutate USER_TYPE + the global skill store; serialise
    // with a local lock. `claude_core::constants::ENV_LOCK` is
    // crate-private and not reachable from claude-tools.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn simplify_registers_unconditionally() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        clear_skills();
        std::env::remove_var("USER_TYPE");
        register_simplify_skill();
        let skills = list_skills();
        assert!(skills.iter().any(|s| s.name == "simplify"));
        clear_skills();
    }

    #[test]
    fn stuck_ant_only() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());

        clear_skills();
        std::env::remove_var("USER_TYPE");
        register_stuck_skill();
        assert!(
            !list_skills().iter().any(|s| s.name == "stuck"),
            "stuck must not register for non-ant users"
        );

        clear_skills();
        std::env::set_var("USER_TYPE", "ant");
        register_stuck_skill();
        assert!(list_skills().iter().any(|s| s.name == "stuck"));

        std::env::remove_var("USER_TYPE");
        clear_skills();
    }

    #[test]
    fn remember_gated_on_ant_and_auto_memory() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());

        // Non-ant: hidden regardless of auto-memory.
        clear_skills();
        std::env::remove_var("USER_TYPE");
        std::env::remove_var("CLAUDE_CODE_DISABLE_AUTO_MEMORY");
        register_remember_skill();
        assert!(!list_skills().iter().any(|s| s.name == "remember"));

        // Ant + auto-memory disabled: hidden (codex CR fix).
        clear_skills();
        std::env::set_var("USER_TYPE", "ant");
        std::env::set_var("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "true");
        register_remember_skill();
        assert!(
            !list_skills().iter().any(|s| s.name == "remember"),
            "remember must not register when auto-memory is off"
        );

        // Ant + auto-memory enabled (default): registered.
        clear_skills();
        std::env::set_var("USER_TYPE", "ant");
        std::env::remove_var("CLAUDE_CODE_DISABLE_AUTO_MEMORY");
        register_remember_skill();
        assert!(list_skills().iter().any(|s| s.name == "remember"));

        std::env::remove_var("USER_TYPE");
        std::env::remove_var("CLAUDE_CODE_DISABLE_AUTO_MEMORY");
        clear_skills();
    }

    #[test]
    fn skills_carry_per_skill_arg_headers() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        clear_skills();
        std::env::set_var("USER_TYPE", "ant");
        std::env::remove_var("CLAUDE_CODE_DISABLE_AUTO_MEMORY");

        register_bundled_skills();
        let skills = list_skills();

        let simplify = skills.iter().find(|s| s.name == "simplify").unwrap();
        assert_eq!(
            simplify.argument_header.as_deref(),
            Some("Additional Focus")
        );

        let stuck = skills.iter().find(|s| s.name == "stuck").unwrap();
        assert_eq!(
            stuck.argument_header.as_deref(),
            Some("User-provided context")
        );

        let remember = skills.iter().find(|s| s.name == "remember").unwrap();
        assert_eq!(
            remember.argument_header.as_deref(),
            Some("Additional context from user")
        );

        std::env::remove_var("USER_TYPE");
        clear_skills();
    }

    #[test]
    fn register_all_is_idempotent() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        clear_skills();
        std::env::set_var("USER_TYPE", "ant");
        std::env::remove_var("CLAUDE_CODE_DISABLE_AUTO_MEMORY");

        register_bundled_skills();
        let first_count = list_skills().len();
        register_bundled_skills();
        let second_count = list_skills().len();
        assert_eq!(first_count, second_count);

        std::env::remove_var("USER_TYPE");
        clear_skills();
    }

    #[test]
    fn registered_prompts_are_non_empty() {
        // Sanity: the included .md files loaded successfully.
        assert!(SIMPLIFY_PROMPT.contains("# Simplify"));
        assert!(STUCK_PROMPT.contains("/stuck"));
        assert!(REMEMBER_PROMPT.contains("# Memory Review"));
        assert!(LOOP_PROMPT.contains("/loop"));
        assert!(LOOP_USAGE.contains("Usage: /loop"));
    }

    #[test]
    fn loop_gated_on_agent_triggers_and_disable_cron() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());

        // Absent: hidden.
        clear_skills();
        std::env::remove_var("AGENT_TRIGGERS");
        std::env::remove_var("CLAUDE_CODE_DISABLE_CRON");
        register_loop_skill();
        assert!(!list_skills().iter().any(|s| s.name == "loop"));

        // AGENT_TRIGGERS truthy + DISABLE_CRON truthy: hidden
        // (codex CR: local kill-switch must be honored).
        clear_skills();
        std::env::set_var("AGENT_TRIGGERS", "1");
        std::env::set_var("CLAUDE_CODE_DISABLE_CRON", "true");
        register_loop_skill();
        assert!(
            !list_skills().iter().any(|s| s.name == "loop"),
            "loop must not register when CLAUDE_CODE_DISABLE_CRON is truthy"
        );

        // AGENT_TRIGGERS truthy + DISABLE_CRON unset: registered.
        clear_skills();
        std::env::set_var("AGENT_TRIGGERS", "1");
        std::env::remove_var("CLAUDE_CODE_DISABLE_CRON");
        register_loop_skill();
        assert!(list_skills().iter().any(|s| s.name == "loop"));

        std::env::remove_var("AGENT_TRIGGERS");
        std::env::remove_var("CLAUDE_CODE_DISABLE_CRON");
        clear_skills();
    }

    /// Prompt text in loop.md references `CronCreate`; the tool
    /// must actually be registered under that exact name (aliases
    /// are fine). This test catches the regression codex CR
    /// flagged — earlier the Rust tool reported
    /// `"ScheduleCron"` and the prompt pointed the model at a
    /// non-existent tool.
    #[test]
    fn loop_prompt_tool_name_matches_registered_tool() {
        let tool = crate::cron_tool::ScheduleCronTool;
        assert_eq!(
            crate::registry::ToolExecutor::name(&tool),
            "CronCreate",
            "loop.md hard-codes `CronCreate`; rename is not allowed"
        );
        assert!(LOOP_PROMPT.contains("CronCreate"));
    }

    #[test]
    fn loop_carries_input_header_and_usage_fallback() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        clear_skills();
        std::env::set_var("AGENT_TRIGGERS", "1");
        std::env::remove_var("CLAUDE_CODE_DISABLE_CRON");

        register_loop_skill();
        let l = list_skills().into_iter().find(|s| s.name == "loop").unwrap();
        assert_eq!(l.argument_header.as_deref(), Some("Input"));
        assert!(l.empty_args_message.as_deref().is_some());
        assert!(l
            .empty_args_message
            .as_deref()
            .unwrap()
            .contains("Usage: /loop"));

        std::env::remove_var("AGENT_TRIGGERS");
        clear_skills();
    }
}
