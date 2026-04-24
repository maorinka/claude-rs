//! Shared constants and path builders for MDM (Mobile Device Management)
//! settings modules.
//!
//! Port of TS `utils/settings/mdm/constants.ts:1-82`.
//!
//! Zero heavy imports (only `std::env`, `dirs`, `std::path`) — safe to
//! pull into the MDM raw-read layer without cycling back into the
//! full settings infrastructure.

use std::path::{Path, PathBuf};

/// macOS preference domain for Claude Code MDM profiles.
pub const MACOS_PREFERENCE_DOMAIN: &str = "com.anthropic.claudecode";

/// Windows registry key paths for Claude Code MDM policies.
///
/// These live under `SOFTWARE\Policies` which is on the WOW64 shared-key
/// list — both 32-bit and 64-bit processes see the same values without
/// redirection. Do NOT move to `SOFTWARE\ClaudeCode`: `SOFTWARE` is
/// redirected and 32-bit processes would silently read from
/// `WOW6432Node`. TS comment at `constants.ts:17-21` references the
/// Microsoft docs page.
pub const WINDOWS_REGISTRY_KEY_PATH_HKLM: &str = r"HKLM\SOFTWARE\Policies\ClaudeCode";
pub const WINDOWS_REGISTRY_KEY_PATH_HKCU: &str = r"HKCU\SOFTWARE\Policies\ClaudeCode";

/// Windows registry value name containing the JSON settings blob.
pub const WINDOWS_REGISTRY_VALUE_NAME: &str = "Settings";

/// Path to macOS `plutil` binary.
pub const PLUTIL_PATH: &str = "/usr/bin/plutil";

/// Arguments for `plutil` to convert a plist to JSON on stdout. The
/// plist path is appended by the caller after this prefix. `--` stops
/// option parsing so a path starting with `-` is treated as a path.
pub const PLUTIL_ARGS_PREFIX: &[&str] = &["-convert", "json", "-o", "-", "--"];

/// Subprocess timeout in milliseconds — applied to `plutil` / `reg query`
/// invocations from the MDM readers so a wedged child doesn't stall
/// startup.
pub const MDM_SUBPROCESS_TIMEOUT_MS: u64 = 5_000;

/// One MDM plist candidate: its absolute path and a human-readable
/// label for telemetry / debug logs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacOsPlistCandidate {
    pub path: PathBuf,
    pub label: &'static str,
}

fn current_username() -> Option<String> {
    // TS uses `userInfo().username`. Rust: prefer `$USER` (Unix convention
    // — the existing crate uses this elsewhere, see `auth/storage.rs`),
    // fall back to `$LOGNAME`. Windows has `USERNAME` but the caller only
    // uses the result to build a macOS path, so Unix-only is fine.
    std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .ok()
        .filter(|s| !s.is_empty())
}

fn is_ant_user() -> bool {
    // TS: `process.env.USER_TYPE === 'ant'`. Matched byte-for-byte —
    // any case/whitespace drift would silently skip the ant-only path.
    matches!(std::env::var("USER_TYPE").as_deref(), Ok("ant"))
}

/// Build the priority-ordered list of macOS plist paths the MDM reader
/// should check. Highest priority first. TS `getMacOSPlistPaths`.
///
/// Evaluates env vars at call time so a test can flip `USER_TYPE`
/// between invocations.
pub fn get_macos_plist_paths() -> Vec<MacOsPlistCandidate> {
    let mut paths = Vec::with_capacity(3);

    if let Some(username) = current_username() {
        paths.push(MacOsPlistCandidate {
            path: Path::new("/Library/Managed Preferences")
                .join(&username)
                .join(format!("{MACOS_PREFERENCE_DOMAIN}.plist")),
            label: "per-user managed preferences",
        });
    }

    paths.push(MacOsPlistCandidate {
        path: Path::new("/Library/Managed Preferences")
            .join(format!("{MACOS_PREFERENCE_DOMAIN}.plist")),
        label: "device-level managed preferences",
    });

    // Ant-only path — lets internal devs test MDM profiles without
    // sudo-writing to /Library. TS gates on `USER_TYPE === 'ant'`; any
    // non-ant build skips this entry so a workstation profile can't
    // override a real MDM deployment.
    if is_ant_user() {
        if let Some(home) = dirs::home_dir() {
            paths.push(MacOsPlistCandidate {
                path: home
                    .join("Library")
                    .join("Preferences")
                    .join(format!("{MACOS_PREFERENCE_DOMAIN}.plist")),
                label: "user preferences (ant-only)",
            });
        }
    }

    paths
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
    fn constants_match_ts() {
        // Every string constant here has a concrete downstream consumer
        // (registry reader, plutil invocation) — drift would silently
        // break MDM detection on one platform.
        assert_eq!(MACOS_PREFERENCE_DOMAIN, "com.anthropic.claudecode");
        assert_eq!(
            WINDOWS_REGISTRY_KEY_PATH_HKLM,
            r"HKLM\SOFTWARE\Policies\ClaudeCode"
        );
        assert_eq!(
            WINDOWS_REGISTRY_KEY_PATH_HKCU,
            r"HKCU\SOFTWARE\Policies\ClaudeCode"
        );
        assert_eq!(WINDOWS_REGISTRY_VALUE_NAME, "Settings");
        assert_eq!(PLUTIL_PATH, "/usr/bin/plutil");
        assert_eq!(PLUTIL_ARGS_PREFIX, &["-convert", "json", "-o", "-", "--"]);
        assert_eq!(MDM_SUBPROCESS_TIMEOUT_MS, 5_000);
    }

    #[test]
    fn device_level_path_always_present() {
        let _g = lock_env();
        std::env::remove_var("USER");
        std::env::remove_var("LOGNAME");
        std::env::remove_var("USER_TYPE");

        let paths = get_macos_plist_paths();
        // Even with zero env, the device-level entry must be there.
        assert!(paths
            .iter()
            .any(|c| c.label == "device-level managed preferences"));
    }

    #[test]
    fn per_user_path_included_when_username_present() {
        let _g = lock_env();
        std::env::set_var("USER", "alice");
        std::env::remove_var("LOGNAME");
        std::env::remove_var("USER_TYPE");

        let paths = get_macos_plist_paths();
        let per_user = paths
            .iter()
            .find(|c| c.label == "per-user managed preferences")
            .expect("expected per-user entry");
        assert!(per_user
            .path
            .to_string_lossy()
            .contains("/Library/Managed Preferences/alice/"));
        std::env::remove_var("USER");
    }

    #[test]
    fn empty_username_skips_per_user_path() {
        let _g = lock_env();
        std::env::set_var("USER", "");
        std::env::remove_var("LOGNAME");
        std::env::remove_var("USER_TYPE");

        let paths = get_macos_plist_paths();
        assert!(!paths
            .iter()
            .any(|c| c.label == "per-user managed preferences"));
        std::env::remove_var("USER");
    }

    #[test]
    fn ant_only_path_gated_by_user_type() {
        let _g = lock_env();
        std::env::remove_var("USER");
        std::env::remove_var("LOGNAME");

        // Non-ant: skipped.
        std::env::set_var("USER_TYPE", "external");
        let paths = get_macos_plist_paths();
        assert!(!paths
            .iter()
            .any(|c| c.label == "user preferences (ant-only)"));

        // Ant: included (assuming home dir resolves, which it does in
        // `cargo test`'s environment).
        std::env::set_var("USER_TYPE", "ant");
        let paths = get_macos_plist_paths();
        assert!(paths
            .iter()
            .any(|c| c.label == "user preferences (ant-only)"));

        std::env::remove_var("USER_TYPE");
    }

    #[test]
    fn priority_order_matches_ts() {
        let _g = lock_env();
        std::env::set_var("USER", "bob");
        std::env::remove_var("LOGNAME");
        std::env::set_var("USER_TYPE", "ant");

        let labels: Vec<&str> = get_macos_plist_paths().iter().map(|c| c.label).collect();
        // TS constants.ts:55-78: per-user → device-level → (ant-only).
        assert_eq!(
            labels,
            vec![
                "per-user managed preferences",
                "device-level managed preferences",
                "user preferences (ant-only)",
            ]
        );

        std::env::remove_var("USER");
        std::env::remove_var("USER_TYPE");
    }

    #[test]
    fn falls_back_to_logname_when_user_unset() {
        let _g = lock_env();
        std::env::remove_var("USER");
        std::env::set_var("LOGNAME", "carol");
        std::env::remove_var("USER_TYPE");

        let paths = get_macos_plist_paths();
        let per_user = paths
            .iter()
            .find(|c| c.label == "per-user managed preferences")
            .expect("expected per-user entry via LOGNAME fallback");
        assert!(per_user.path.to_string_lossy().contains("/carol/"));

        std::env::remove_var("LOGNAME");
    }
}
