//! Memoised check for whether `git` is on PATH.
//!
//! Port of TS `utils/plugins/gitAvailability.ts:1-70`.
//!
//! Used to gate features that require git (notably installing
//! GitHub-based marketplaces). Checks PATH via `which` — never execs
//! the binary. TS comment explains why: on macOS the `/usr/bin/git`
//! xcrun shim exists in PATH even without Xcode CLT installed. A PATH
//! lookup passes, then the first real `git …` call dies with
//! `xcrun: error: invalid active developer path`. Callers that hit
//! that should call [`mark_git_unavailable`] so the rest of the session
//! short-circuits.

use once_cell::sync::Lazy;
use std::sync::Mutex;

/// `Some(true)` → memoised-available, `Some(false)` → memoised-unavailable
/// (either by a real check that failed or by `mark_git_unavailable`),
/// `None` → not checked yet. Wrapped in a Mutex so both the lazy check
/// and the force-unavailable path can mutate under concurrent access.
static CACHE: Lazy<Mutex<Option<bool>>> = Lazy::new(|| Mutex::new(None));

fn lock() -> std::sync::MutexGuard<'static, Option<bool>> {
    CACHE.lock().unwrap_or_else(|p| p.into_inner())
}

fn is_on_path(cmd: &str) -> bool {
    // `which::which` walks PATH + applies platform executable-bit / PATHEXT
    // rules without executing the target. Errors (not found, permission
    // denied) map to false — the TS `try/catch` around `which` does the
    // same (gitAvailability.ts:22-26).
    which::which(cmd).is_ok()
}

/// Returns `true` iff `git` is reachable via PATH. Memoised for the
/// process lifetime (TS uses `lodash.memoize` for the same reason — git
/// availability doesn't change during a session).
pub fn check_git_available() -> bool {
    let mut guard = lock();
    if let Some(cached) = *guard {
        return cached;
    }
    let result = is_on_path("git");
    *guard = Some(result);
    result
}

/// Force the memoised check to return `false` for the rest of the
/// session. Call this when a real `git …` invocation fails in a way
/// that indicates the binary is on PATH but not actually runnable — the
/// macOS xcrun shim case called out in the TS docs.
pub fn mark_git_unavailable() {
    *lock() = Some(false);
}

/// Clear the memoised check. Exposed for testing only — analogous to
/// `clearGitAvailabilityCache` in TS.
#[doc(hidden)]
pub fn clear_git_availability_cache() {
    *lock() = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    // Tests mutate the module-global cache and PATH, so they must serialise.
    static TEST_LOCK: StdMutex<()> = StdMutex::new(());

    fn guard() -> std::sync::MutexGuard<'static, ()> {
        TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner())
    }

    #[test]
    fn check_memoises_result() {
        let _g = guard();
        clear_git_availability_cache();
        let first = check_git_available();
        let second = check_git_available();
        assert_eq!(first, second);
    }

    #[test]
    fn mark_unavailable_overrides_check() {
        let _g = guard();
        clear_git_availability_cache();
        // Whatever the real state, force-unavailable must stick.
        mark_git_unavailable();
        assert!(!check_git_available());
        // Still unavailable on repeated calls — TS's xcrun workaround
        // requires the stickiness to survive until session end.
        assert!(!check_git_available());
    }

    #[test]
    fn clear_resets_cache() {
        let _g = guard();
        mark_git_unavailable();
        assert!(!check_git_available());
        clear_git_availability_cache();
        // After clear the real check runs again — result depends on the
        // host PATH, so just assert the cache populated (non-None).
        let _ = check_git_available();
        assert!(lock().is_some());
    }

    #[test]
    fn availability_matches_real_which() {
        // Sanity-check against `which` directly: no monkey-business in
        // the memoisation wrapper.
        let _g = guard();
        clear_git_availability_cache();
        assert_eq!(check_git_available(), which::which("git").is_ok());
    }
}
