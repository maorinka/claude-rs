//! Team memory path helpers.
//!
//! Port of the pure-path half of TS `src/memdir/teamMemPaths.ts`.
//! The TS file also contains async realpath + symlink-escape
//! validation (`PathTraversalError`, `validateTeamMemWritePath`,
//! `isRealPathWithinTeamDir`) — those need tokio::fs and the
//! cross-platform symlink-resolution handling that isn't on the
//! Rust side yet. This module ports the synchronous shape-level
//! checks, which is what TS `teamMemSecretGuard.ts` depends on.
//!
//! Team memory lives as a subdirectory of auto-memory scoped
//! per-project:
//! `<memoryBase>/projects/<sanitized-cwd>/memory/team/`

use super::paths::{auto_memory_enabled, get_auto_mem_path};
use std::path::{Path, PathBuf};

/// Directory name under the auto-mem root that holds team memory.
/// TS hard-codes `'team'`; kept as a constant here so the secret
/// guard and any future team-mem helpers share it.
pub const TEAM_MEM_DIRNAME: &str = "team";

/// `<autoMemPath(cwd)>/team/`. Matches TS `getTeamMemPath()` —
/// TS appends a trailing separator + NFC-normalises; Rust's
/// `PathBuf` handles separators per-platform and we skip the
/// trailing-separator step because callers use `starts_with`
/// (which handles separator boundaries correctly via
/// [`std::path::Path::starts_with`]).
pub fn get_team_mem_path(cwd: &Path) -> PathBuf {
    get_auto_mem_path(cwd).join(TEAM_MEM_DIRNAME)
}

/// `<autoMemPath(cwd)>/team/MEMORY.md`. Matches TS
/// `getTeamMemEntrypoint()`.
pub fn get_team_mem_entrypoint(cwd: &Path) -> PathBuf {
    get_team_mem_path(cwd).join("MEMORY.md")
}

/// Whether team memory features are enabled. Matches TS
/// `isTeamMemoryEnabled()`: requires auto-memory on AND the
/// `TEAMMEM` feature flag (env truthy). TS also consults a
/// GrowthBook flag (`tengu_herring_clock` defaulting false);
/// GrowthBook isn't on the Rust side, so the env gate is the
/// sole feature check.
///
/// Behaviour: `TEAMMEM` truthy is REQUIRED — there's no
/// fleet-wide kill-switch to flip, so the most conservative
/// default (off unless explicitly opted in) matches the TS
/// default-false GrowthBook flag.
pub fn is_team_memory_enabled() -> bool {
    if !auto_memory_enabled() {
        return false;
    }
    crate::errors_util::is_env_truthy("TEAMMEM")
}

/// Check whether a resolved absolute path is within the team
/// memory directory for `cwd`. Matches TS `isTeamMemPath()`:
/// resolves `..` segments via `Path::components` then prefix-
/// checks against the team dir. Does NOT resolve symlinks —
/// callers that need symlink-escape protection should layer on
/// [`std::fs::canonicalize`] plus the TS
/// `validateTeamMemWritePath` flow (deferred here).
///
/// Prefix-attack protection is delegated to
/// [`Path::starts_with`], which requires a separator boundary
/// (so `/foo/team-evil` doesn't match `/foo/team`).
pub fn is_team_mem_path(file_path: &Path, cwd: &Path) -> bool {
    let resolved = resolve_dot_segments(file_path);
    let team_dir = get_team_mem_path(cwd);
    resolved.starts_with(team_dir)
}

/// Eliminate `.` and `..` segments from `p` without touching the
/// filesystem. Port of TS `path.resolve(filePath)`'s
/// normalisation step (TS resolve ALSO converts relative to
/// absolute; we leave relative paths as-is so callers that want
/// absolutisation can do it explicitly).
fn resolve_dot_segments(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in p.components() {
        use std::path::Component;
        match comp {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::ENV_LOCK;
    use std::path::Path;

    #[test]
    fn team_mem_path_is_auto_mem_plus_team() {
        let cwd = Path::new("/Users/alex/proj");
        let team = get_team_mem_path(cwd);
        let auto = get_auto_mem_path(cwd);
        assert_eq!(team, auto.join("team"));
    }

    #[test]
    fn team_mem_entrypoint_is_memory_md_inside_team_dir() {
        let cwd = Path::new("/Users/alex/proj");
        let ep = get_team_mem_entrypoint(cwd);
        assert!(ep.ends_with("team/MEMORY.md"));
    }

    #[test]
    fn is_team_mem_path_accepts_direct_child() {
        let cwd = Path::new("/Users/alex/proj");
        let team = get_team_mem_path(cwd);
        let child = team.join("shared.md");
        assert!(is_team_mem_path(&child, cwd));
    }

    #[test]
    fn is_team_mem_path_rejects_sibling() {
        // Prefix-attack protection: "/foo/team-evil" must NOT
        // match "/foo/team" per TS comments.
        let cwd = Path::new("/Users/alex/proj");
        let team = get_team_mem_path(cwd);
        // Build a sibling that starts with the team dir name
        // but isn't inside it.
        let team_str = team.to_string_lossy().to_string();
        let evil = PathBuf::from(format!("{team_str}-evil/file.md"));
        assert!(!is_team_mem_path(&evil, cwd));
    }

    #[test]
    fn is_team_mem_path_resolves_dot_dot() {
        let cwd = Path::new("/Users/alex/proj");
        let team = get_team_mem_path(cwd);
        // e.g. <team>/subdir/../shared.md → <team>/shared.md — in.
        let inside = team.join("subdir").join("..").join("shared.md");
        assert!(is_team_mem_path(&inside, cwd));
        // e.g. <team>/../escape.md → outside the team dir.
        let outside = team.join("..").join("escape.md");
        assert!(!is_team_mem_path(&outside, cwd));
    }

    #[test]
    fn is_team_memory_enabled_off_when_auto_memory_off() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        std::env::set_var("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "true");
        std::env::set_var("TEAMMEM", "1");
        assert!(!is_team_memory_enabled());
        std::env::remove_var("CLAUDE_CODE_DISABLE_AUTO_MEMORY");
        std::env::remove_var("TEAMMEM");
    }

    #[test]
    fn is_team_memory_enabled_off_when_teammem_env_off() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        std::env::remove_var("CLAUDE_CODE_DISABLE_AUTO_MEMORY");
        std::env::remove_var("TEAMMEM");
        assert!(!is_team_memory_enabled());
    }

    #[test]
    fn is_team_memory_enabled_on_when_both_set() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        std::env::remove_var("CLAUDE_CODE_DISABLE_AUTO_MEMORY");
        std::env::set_var("TEAMMEM", "1");
        assert!(is_team_memory_enabled());
        std::env::remove_var("TEAMMEM");
    }
}
