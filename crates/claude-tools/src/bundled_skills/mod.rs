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
//! - `remember` — ant-only
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

use crate::skill_tool::register_skill_with_arg_header;
use claude_core::memdir::auto_memory_enabled;
use claude_core::user_type;

const SIMPLIFY_PROMPT: &str = include_str!("simplify.md");
const STUCK_PROMPT: &str = include_str!("stuck.md");
const REMEMBER_PROMPT: &str = include_str!("remember.md");

/// Register every bundled skill whose gate passes for the current
/// user type. Idempotent: `register_skill` replaces by name, so
/// calling this twice is harmless.
pub fn register_bundled_skills() {
    register_simplify_skill();
    register_stuck_skill();
    register_remember_skill();
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
    }
}
