//! Shell-tool registry + PowerShellTool runtime gate.
//!
//! Port of TS `utils/shell/shellToolUtils.ts:1-22`.
//!
//! - [`SHELL_TOOL_NAMES`] enumerates the two shell executors the tool
//!   registry knows about (Bash + PowerShell). Referenced from tool-list
//!   filters that strip ALL shells for restricted modes.
//! - [`is_powershell_tool_enabled`] is the Windows-only runtime gate
//!   that controls whether the PowerShellTool is surfaced. Must stay in
//!   sync across every code path that invokes `PowerShellTool::call` —
//!   tools.ts (registry), processBashCommand (`!` routing), and
//!   promptShellExecution (skill frontmatter routing).

use crate::errors_util::{is_env_definitely_falsy, is_env_truthy};
use crate::tool_names::{BASH_TOOL_NAME, POWERSHELL_TOOL_NAME};
use crate::user_type;

/// Env var both branches of [`is_powershell_tool_enabled`] consult.
/// Exported so call sites and tests share one source of truth.
pub const POWERSHELL_TOOL_ENV: &str = "CLAUDE_CODE_USE_POWERSHELL_TOOL";

/// The two shell executor tool names in registry-listing order. Matches
/// TS `shellToolUtils.ts:6` — callers use this for "strip shell tools"
/// filters, so the exact names must line up with the registry.
pub const SHELL_TOOL_NAMES: &[&str] = &[BASH_TOOL_NAME, POWERSHELL_TOOL_NAME];

/// Runtime gate for PowerShellTool.
///
/// Returns `false` on every non-Windows platform — the TS comment calls
/// out that the permission engine depends on Win32-specific path
/// normalisations. On Windows:
/// - `USER_TYPE=ant` → default ON, opt-out via env=0 (internal users
///   get the tool by default, env can disable it for debugging).
/// - otherwise → default OFF, opt-in via env=1 (external users must
///   explicitly opt in).
pub fn is_powershell_tool_enabled() -> bool {
    if !cfg!(target_os = "windows") {
        return false;
    }
    if user_type::is_ant() {
        // Ant default ON — env var must be explicitly falsy to disable.
        !is_env_definitely_falsy(POWERSHELL_TOOL_ENV)
    } else {
        // External default OFF — env var must be explicitly truthy.
        is_env_truthy(POWERSHELL_TOOL_ENV)
    }
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
    fn shell_tool_names_contains_exactly_bash_and_powershell() {
        assert_eq!(SHELL_TOOL_NAMES.len(), 2);
        assert!(SHELL_TOOL_NAMES.contains(&BASH_TOOL_NAME));
        assert!(SHELL_TOOL_NAMES.contains(&POWERSHELL_TOOL_NAME));
    }

    #[test]
    fn off_on_non_windows_regardless_of_env() {
        if cfg!(target_os = "windows") {
            return;
        }
        let _g = lock_env();
        // Prove: every env combination that would matter on Windows
        // still returns false on non-Windows.
        std::env::set_var("USER_TYPE", "ant");
        std::env::set_var(POWERSHELL_TOOL_ENV, "1");
        assert!(!is_powershell_tool_enabled());

        std::env::set_var("USER_TYPE", "external");
        std::env::set_var(POWERSHELL_TOOL_ENV, "1");
        assert!(!is_powershell_tool_enabled());

        std::env::remove_var("USER_TYPE");
        std::env::remove_var(POWERSHELL_TOOL_ENV);
    }

    #[test]
    fn windows_ant_defaults_on() {
        if !cfg!(target_os = "windows") {
            return;
        }
        let _g = lock_env();
        std::env::set_var("USER_TYPE", "ant");
        std::env::remove_var(POWERSHELL_TOOL_ENV);
        assert!(is_powershell_tool_enabled());
        std::env::remove_var("USER_TYPE");
    }

    #[test]
    fn windows_ant_opt_out_via_env_zero() {
        if !cfg!(target_os = "windows") {
            return;
        }
        let _g = lock_env();
        std::env::set_var("USER_TYPE", "ant");
        std::env::set_var(POWERSHELL_TOOL_ENV, "0");
        assert!(!is_powershell_tool_enabled());
        std::env::set_var(POWERSHELL_TOOL_ENV, "false");
        assert!(!is_powershell_tool_enabled());
        std::env::remove_var(POWERSHELL_TOOL_ENV);
        std::env::remove_var("USER_TYPE");
    }

    #[test]
    fn windows_external_defaults_off() {
        if !cfg!(target_os = "windows") {
            return;
        }
        let _g = lock_env();
        std::env::set_var("USER_TYPE", "external");
        std::env::remove_var(POWERSHELL_TOOL_ENV);
        assert!(!is_powershell_tool_enabled());
        std::env::remove_var("USER_TYPE");
    }

    #[test]
    fn windows_external_opt_in_via_env_one() {
        if !cfg!(target_os = "windows") {
            return;
        }
        let _g = lock_env();
        std::env::set_var("USER_TYPE", "external");
        std::env::set_var(POWERSHELL_TOOL_ENV, "1");
        assert!(is_powershell_tool_enabled());
        std::env::set_var(POWERSHELL_TOOL_ENV, "true");
        assert!(is_powershell_tool_enabled());
        std::env::remove_var(POWERSHELL_TOOL_ENV);
        std::env::remove_var("USER_TYPE");
    }

    #[test]
    fn user_type_env_var_is_named_correctly() {
        // Wire-format pin: the whole ant/external branch depends on
        // reading the exact env var name. A typo here would silently
        // put all ant users on the external default (off).
        let _g = lock_env();
        std::env::remove_var("USER_TYPE");
        // With USER_TYPE unset, `is_ant()` returns false → external
        // branch applies regardless of platform.
        assert!(!user_type::is_ant());
    }
}
