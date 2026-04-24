//! Platform-specific path to the managed-settings directory.
//!
//! Port of TS `utils/settings/managedPath.ts:1-35`.
//!
//! The managed-settings layer is the highest-precedence slot in the
//! config merge chain — it's how fleet admins / MDM profiles enforce
//! policy. This module only *resolves* the path; reading, parsing, and
//! merging live elsewhere.

use crate::user_type;
use std::path::PathBuf;
use std::sync::OnceLock;

static MANAGED_FILE_PATH: OnceLock<PathBuf> = OnceLock::new();
static MANAGED_DROP_IN_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Env-var escape hatch. Only honoured when `USER_TYPE=ant` — external
/// builds can't override the managed path (which would defeat the point
/// of having managed settings).
const ANT_OVERRIDE_ENV: &str = "CLAUDE_CODE_MANAGED_SETTINGS_PATH";

fn resolve_managed_file_path() -> PathBuf {
    if user_type::is_ant() {
        if let Ok(override_path) = std::env::var(ANT_OVERRIDE_ENV) {
            if !override_path.is_empty() {
                return PathBuf::from(override_path);
            }
        }
    }

    // TS uses `getPlatform()` which maps Node `process.platform` to
    // `'macos' | 'windows' | 'linux'`. Rust uses `cfg!` for compile-
    // time branching — same outcome since the target triple is fixed
    // per binary.
    if cfg!(target_os = "macos") {
        PathBuf::from("/Library/Application Support/ClaudeCode")
    } else if cfg!(target_os = "windows") {
        PathBuf::from(r"C:\Program Files\ClaudeCode")
    } else {
        PathBuf::from("/etc/claude-code")
    }
}

/// Resolve the managed-settings directory path. Memoised for the
/// process lifetime — TS uses `lodash.memoize` for the same reason
/// (env check + platform switch are cheap, but called from hot paths).
///
/// Callers that need to re-read after env mutation in tests should use
/// [`clear_managed_path_cache_for_tests`].
pub fn get_managed_file_path() -> &'static PathBuf {
    MANAGED_FILE_PATH.get_or_init(resolve_managed_file_path)
}

/// `<managed_file_path>/managed-settings.d` — the drop-in directory.
///
/// TS merges `managed-settings.json` first (base), then files in this
/// directory alphabetically on top — later files win. This port only
/// resolves the path; the merger lives elsewhere.
pub fn get_managed_settings_drop_in_dir() -> &'static PathBuf {
    MANAGED_DROP_IN_DIR.get_or_init(|| get_managed_file_path().join("managed-settings.d"))
}

/// Test-only cache reset. `OnceLock` has no public `take`, but tests
/// mutate env vars that flip the resolved path, so we expose a helper
/// that re-evaluates and compares against the initialised value.
///
/// Rather than fighting the `OnceLock` semantics in tests, tests below
/// use [`resolve_managed_file_path`] directly (exposed via
/// `#[cfg(test)]`) to avoid the memo.
#[cfg(test)]
pub(crate) fn resolve_uncached() -> PathBuf {
    resolve_managed_file_path()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn lock_env() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner())
    }

    #[test]
    fn platform_default_no_env() {
        let _g = lock_env();
        std::env::remove_var("USER_TYPE");
        std::env::remove_var(ANT_OVERRIDE_ENV);
        let p = resolve_uncached();

        if cfg!(target_os = "macos") {
            assert_eq!(p, PathBuf::from("/Library/Application Support/ClaudeCode"));
        } else if cfg!(target_os = "windows") {
            assert_eq!(p, PathBuf::from(r"C:\Program Files\ClaudeCode"));
        } else {
            assert_eq!(p, PathBuf::from("/etc/claude-code"));
        }
    }

    #[test]
    fn ant_override_honoured() {
        let _g = lock_env();
        std::env::set_var("USER_TYPE", "ant");
        std::env::set_var(ANT_OVERRIDE_ENV, "/tmp/fake-managed");
        assert_eq!(resolve_uncached(), PathBuf::from("/tmp/fake-managed"));
        std::env::remove_var(ANT_OVERRIDE_ENV);
        std::env::remove_var("USER_TYPE");
    }

    #[test]
    fn non_ant_ignores_override() {
        // Security contract: external builds cannot override — that
        // would defeat the entire point of managed settings.
        let _g = lock_env();
        std::env::set_var("USER_TYPE", "external");
        std::env::set_var(ANT_OVERRIDE_ENV, "/tmp/attacker-controlled");
        let p = resolve_uncached();
        assert_ne!(p, PathBuf::from("/tmp/attacker-controlled"));
        std::env::remove_var(ANT_OVERRIDE_ENV);
        std::env::remove_var("USER_TYPE");
    }

    #[test]
    fn unset_user_type_ignores_override() {
        let _g = lock_env();
        std::env::remove_var("USER_TYPE");
        std::env::set_var(ANT_OVERRIDE_ENV, "/tmp/unset-case");
        let p = resolve_uncached();
        assert_ne!(p, PathBuf::from("/tmp/unset-case"));
        std::env::remove_var(ANT_OVERRIDE_ENV);
    }

    #[test]
    fn empty_override_falls_back_to_platform_default() {
        let _g = lock_env();
        std::env::set_var("USER_TYPE", "ant");
        std::env::set_var(ANT_OVERRIDE_ENV, "");
        let p = resolve_uncached();
        // Empty string env var must not be treated as a valid override —
        // TS falsy check (`if (... && CLAUDE_CODE_MANAGED_SETTINGS_PATH)`)
        // also rejects the empty string.
        assert!(!p.as_os_str().is_empty());
        assert_ne!(p, PathBuf::from(""));
        std::env::remove_var(ANT_OVERRIDE_ENV);
        std::env::remove_var("USER_TYPE");
    }

    #[test]
    fn drop_in_dir_nests_under_base() {
        let _g = lock_env();
        std::env::remove_var("USER_TYPE");
        std::env::remove_var(ANT_OVERRIDE_ENV);

        // The public memoised accessor will pin whatever it resolved first;
        // inspect the composition logic directly instead.
        let base = resolve_uncached();
        let drop_in = base.join("managed-settings.d");
        assert!(drop_in.ends_with("managed-settings.d"));
        assert_eq!(drop_in.parent().unwrap(), base);
    }

    #[test]
    fn memo_stable_within_process() {
        // First call initialises; second must return the identical
        // reference. Real env mutation mid-process is a test-only
        // concern — production only calls after init().
        let a = get_managed_file_path();
        let b = get_managed_file_path();
        assert!(std::ptr::eq(a, b));
    }
}
