//! Central `USER_TYPE` env reader.
//!
//! Port of the TS `process.env.USER_TYPE` checks scattered across
//! ~168 files. TS equality-checks on the literal strings `'ant'` and
//! (in a few places) `'external'`, treating absence as a distinct
//! state from an explicit `'external'` value. Two TS call sites
//! rely on that distinction — `services/api/withRetry.ts` (only
//! throws the 529 overload error for explicit external users; not
//! for unset or other values) and `services/PromptSuggestion/
//! promptSuggestion.ts` (rate-limit gate only applies to external).
//! So the Rust enum keeps three variants rather than collapsing
//! non-ant into `External`.
//!
//! Build-time `--define` substitution in TS is what enables DCE of
//! ant-only branches in external builds. Rust reads the env at
//! runtime — cheap (one `std::env::var`), not cached, so updates
//! via `std::env::set_var` in tests take effect immediately.

/// Discrete classifier for the `USER_TYPE` env var. Matches TS:
/// - `'ant'` → internal / Anthropic-employee build.
/// - `'external'` → explicit external user (distinct from unset).
/// - anything else OR unset → `Other`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UserType {
    Ant,
    External,
    Other,
}

/// Read `USER_TYPE` and classify. Fresh lookup each call — matches
/// TS which re-reads `process.env.USER_TYPE` on every check (no
/// caching layer).
pub fn current() -> UserType {
    match std::env::var("USER_TYPE").ok().as_deref() {
        Some("ant") => UserType::Ant,
        Some("external") => UserType::External,
        _ => UserType::Other,
    }
}

/// `process.env.USER_TYPE === 'ant'` — the predicate used by ~160
/// of the 168 TS call sites. Gates ant-only features, undercover
/// mode, and the bundled-in internal-codename handling.
pub fn is_ant() -> bool {
    matches!(current(), UserType::Ant)
}

/// `process.env.USER_TYPE === 'external'`. Used by:
/// - [`services/api/withRetry.ts`](claude-code-leaked/src/services/api/withRetry.ts)
///   to throw a custom 529 overload error only for explicit
///   external users (not for unset or other values).
/// - [`services/PromptSuggestion/promptSuggestion.ts`](claude-code-leaked/src/services/PromptSuggestion/promptSuggestion.ts)
///   to gate the rate-limit reason only on explicit externals.
///
/// Returns `false` when `USER_TYPE` is unset, matching TS's
/// strict-equality check.
pub fn is_external() -> bool {
    matches!(current(), UserType::External)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::ENV_LOCK;

    #[test]
    fn ant_string_is_ant() {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::set_var("USER_TYPE", "ant");
        assert_eq!(current(), UserType::Ant);
        assert!(is_ant());
        assert!(!is_external());
        std::env::remove_var("USER_TYPE");
    }

    #[test]
    fn external_string_is_external() {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::set_var("USER_TYPE", "external");
        assert_eq!(current(), UserType::External);
        assert!(!is_ant());
        assert!(is_external());
        std::env::remove_var("USER_TYPE");
    }

    #[test]
    fn unset_is_other() {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::remove_var("USER_TYPE");
        assert_eq!(current(), UserType::Other);
        assert!(!is_ant());
        assert!(!is_external());
    }

    #[test]
    fn empty_string_is_other() {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::set_var("USER_TYPE", "");
        assert_eq!(current(), UserType::Other);
        assert!(!is_ant());
        assert!(!is_external());
        std::env::remove_var("USER_TYPE");
    }

    #[test]
    fn unknown_value_is_other() {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::set_var("USER_TYPE", "internal");
        assert_eq!(current(), UserType::Other);
        std::env::set_var("USER_TYPE", "Ant"); // case matters
        assert_eq!(current(), UserType::Other);
        std::env::remove_var("USER_TYPE");
    }

    #[test]
    fn ant_uppercase_is_not_ant() {
        let _g = ENV_LOCK.lock().unwrap();
        // TS uses `===` — exact case-sensitive match.
        std::env::set_var("USER_TYPE", "ANT");
        assert!(!is_ant());
        std::env::remove_var("USER_TYPE");
    }
}
