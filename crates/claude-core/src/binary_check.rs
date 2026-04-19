//! Session-cached check for whether a named binary is installed and
//! resolvable on the user's `PATH`.
//!
//! Ports `src/utils/binaryCheck.ts`. TS uses the npm `which` package;
//! Rust hand-rolls the lookup rather than pull a dep for this 50-LOC
//! utility. Parity points:
//!   - Empty / whitespace-only command → `false` (TS early-returns at
//!     binaryCheck.ts:16-19).
//!   - Trim before cache lookup and before resolution (TS
//!     binaryCheck.ts:22-30).
//!   - Cache the trimmed form, not the raw input.
//!   - Cross-platform: Unix checks the executable bit; Windows tries
//!     `PATHEXT` extensions. Both mirror the `which` npm package's
//!     behaviour.
//!
//! The cache is process-global and unbounded (matches TS session
//! cache — callers clear via `clear_binary_cache` in tests).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// Trimmed-command → installed? Cache. Mirrors the TS module-level
/// `binaryCache` at `binaryCheck.ts:5`.
fn cache() -> &'static Mutex<HashMap<String, bool>> {
    static CELL: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Return `true` if `command` is installed and resolvable via `PATH`.
/// Empty/whitespace-only input returns `false` without touching the
/// filesystem. Matches `isBinaryInstalled` at `binaryCheck.ts:14-46`.
pub fn is_binary_installed(command: &str) -> bool {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return false;
    }

    if let Ok(guard) = cache().lock() {
        if let Some(&hit) = guard.get(trimmed) {
            return hit;
        }
    }

    let exists = which(trimmed).is_some();

    if let Ok(mut guard) = cache().lock() {
        guard.insert(trimmed.to_string(), exists);
    }

    exists
}

/// Drop the process-wide binary-installed cache. TS calls this
/// from tests at `binaryCheck.ts:51-53`.
pub fn clear_binary_cache() {
    if let Ok(mut guard) = cache().lock() {
        guard.clear();
    }
}

/// Resolve `cmd` against the user's `PATH`, returning the first
/// matching executable path (or `None` if not found).
///
/// - Unix: `PATH` entries are split on `:`. A candidate matches when
///   the file exists and has at least one executable bit set. An
///   absolute / `.`-prefixed `cmd` is checked as-is (same bit check).
/// - Windows: `PATH` splits on `;`. For each dir, `cmd` is tried
///   verbatim and then with each extension in `PATHEXT`
///   (default: `.COM;.EXE;.BAT;.CMD`). No executable-bit check —
///   Windows uses the extension to decide.
fn which(cmd: &str) -> Option<PathBuf> {
    // If the caller gave us a path (contains a separator), resolve
    // directly without walking PATH. `which` npm does the same.
    if cmd.contains(std::path::MAIN_SEPARATOR) || cmd.contains('/') {
        let p = PathBuf::from(cmd);
        return if is_executable_candidate(&p) { Some(p) } else { None };
    }

    let path_var = std::env::var_os("PATH")?;

    // Windows parity with npm `which`: the current working directory
    // is searched *before* `PATH`. Unix deliberately does not — a
    // bare `ls` never resolves from CWD, only from `PATH`.
    #[cfg(windows)]
    let search_dirs: Vec<std::path::PathBuf> = std::iter::once(
        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
    )
    .chain(std::env::split_paths(&path_var))
    .collect();
    #[cfg(not(windows))]
    let search_dirs: Vec<std::path::PathBuf> = std::env::split_paths(&path_var).collect();

    for dir in search_dirs {
        if dir.as_os_str().is_empty() {
            continue;
        }
        let base = dir.join(cmd);

        #[cfg(windows)]
        {
            // Try the literal name first in case it already has an
            // extension (PATHEXT match is case-insensitive; we skip
            // that nuance and rely on NTFS case-insensitivity).
            if is_executable_candidate(&base) {
                return Some(base);
            }
            if let Some(pathext) = std::env::var_os("PATHEXT") {
                for ext in std::env::split_paths(&pathext) {
                    // split_paths uses ';' on Windows and treats each
                    // entry as a path. `.EXE` becomes PathBuf(".EXE") —
                    // good enough for our extension match.
                    let ext_str = ext.to_string_lossy();
                    let mut candidate = base.clone();
                    let mut fname = candidate
                        .file_name()
                        .map(|s| s.to_os_string())
                        .unwrap_or_default();
                    fname.push(ext_str.as_ref());
                    candidate.set_file_name(fname);
                    if is_executable_candidate(&candidate) {
                        return Some(candidate);
                    }
                }
            } else {
                for ext in [".COM", ".EXE", ".BAT", ".CMD"] {
                    let mut candidate = base.clone();
                    let mut fname = candidate
                        .file_name()
                        .map(|s| s.to_os_string())
                        .unwrap_or_default();
                    fname.push(ext);
                    candidate.set_file_name(fname);
                    if is_executable_candidate(&candidate) {
                        return Some(candidate);
                    }
                }
            }
        }

        #[cfg(not(windows))]
        {
            if is_executable_candidate(&base) {
                return Some(base);
            }
        }
    }

    None
}

/// Does `p` refer to a file we could exec?  On Unix we require the
/// user/group/other exec bit. On Windows we only require existence —
/// the extension guards the executability decision.
fn is_executable_candidate(p: &std::path::Path) -> bool {
    let md = match std::fs::metadata(p) {
        Ok(m) => m,
        Err(_) => return false,
    };
    if !md.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        md.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        // Existence + is_file is sufficient on Windows; extension
        // handling happens in the PATH walk.
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Cache + env mutation are global; serialise.
    static T_LOCK: Mutex<()> = Mutex::new(());

    /// Shell is always present on Unix test runners.
    #[test]
    #[cfg(unix)]
    fn finds_sh() {
        let _g = T_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        clear_binary_cache();
        assert!(is_binary_installed("sh"), "sh should be on PATH");
    }

    #[test]
    fn empty_and_whitespace_return_false() {
        let _g = T_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        clear_binary_cache();
        assert!(!is_binary_installed(""));
        assert!(!is_binary_installed("   "));
        assert!(!is_binary_installed("\t\n"));
    }

    #[test]
    fn missing_binary_returns_false() {
        let _g = T_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        clear_binary_cache();
        // A name nobody ships.
        let unlikely = "claude_rs_deliberately_absent_xyzzy";
        assert!(!is_binary_installed(unlikely));
    }

    #[test]
    fn caches_result_across_calls() {
        let _g = T_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        clear_binary_cache();
        let unlikely = "claude_rs_cache_probe_abcdef";
        assert!(!is_binary_installed(unlikely));
        // The second call should be a cache hit — we can't directly
        // observe that, but we can verify the cache entry exists.
        let has = cache()
            .lock()
            .map(|g| g.contains_key(unlikely))
            .unwrap_or(false);
        assert!(has, "missing result must still be cached");
    }

    #[test]
    fn trims_before_caching_and_lookup() {
        let _g = T_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        clear_binary_cache();
        let _ = is_binary_installed("  claude_rs_trim_probe_zzz  ");
        let trimmed_present = cache()
            .lock()
            .map(|g| g.contains_key("claude_rs_trim_probe_zzz"))
            .unwrap_or(false);
        let untrimmed_absent = cache()
            .lock()
            .map(|g| !g.contains_key("  claude_rs_trim_probe_zzz  "))
            .unwrap_or(false);
        assert!(trimmed_present, "cache key must be the trimmed form");
        assert!(untrimmed_absent, "raw whitespace-wrapped key must not be cached");
    }

    #[test]
    fn clear_binary_cache_empties_state() {
        let _g = T_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _ = is_binary_installed("claude_rs_clear_probe_qrs");
        let filled = cache().lock().map(|g| !g.is_empty()).unwrap_or(false);
        assert!(filled, "setup: cache should have at least one entry");
        clear_binary_cache();
        let emptied = cache().lock().map(|g| g.is_empty()).unwrap_or(false);
        assert!(emptied, "clear_binary_cache() must drop every entry");
    }

    /// An absolute or relative path with a separator must be checked
    /// directly, not looked up on PATH. Mirrors npm `which` which
    /// short-circuits on paths that "look like" paths.
    #[test]
    #[cfg(unix)]
    fn absolute_path_bypasses_path_walk() {
        let _g = T_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        clear_binary_cache();
        // /bin/sh is present on every Unix.
        assert!(is_binary_installed("/bin/sh"));
        // Absolute path to nonexistent file must still be false.
        assert!(!is_binary_installed("/definitely/not/here/xyzzy"));
    }

    /// A file that exists but has no exec bit set must not resolve.
    /// Guards the TS parity with npm `which`, which also requires
    /// the executable bit on Unix. Mode `0o644` is the standard
    /// non-executable regular-file permission.
    #[test]
    #[cfg(unix)]
    fn non_executable_regular_file_is_rejected() {
        use std::os::unix::fs::PermissionsExt;
        let _g = T_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        clear_binary_cache();
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("not_exec");
        std::fs::write(&p, b"#!/bin/sh\necho hi\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!is_binary_installed(p.to_str().unwrap()));
        // Flip exec bit: now it resolves.
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        clear_binary_cache();
        assert!(is_binary_installed(p.to_str().unwrap()));
    }
}
