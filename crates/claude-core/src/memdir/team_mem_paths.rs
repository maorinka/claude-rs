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

/// Check whether a path resolves to a location within the team
/// memory directory for `cwd`. Matches TS `isTeamMemPath()`:
/// `path.resolve(filePath).startsWith(getTeamMemPath())`.
///
/// Relative `file_path` is anchored to `cwd` before resolution,
/// mirroring TS's `path.resolve()` behaviour (TS uses
/// `process.cwd()` implicitly; we take `cwd` as a parameter since
/// Rust has no ambient cwd). Dot segments (`.`, `..`) are then
/// eliminated. Does NOT resolve symlinks — callers needing
/// symlink-escape protection should layer on
/// [`std::fs::canonicalize`] + the TS `validateTeamMemWritePath`
/// flow (deferred).
///
/// Prefix-attack protection is delegated to
/// [`Path::starts_with`], which requires a separator boundary
/// (so `/foo/team-evil` doesn't match `/foo/team`).
pub fn is_team_mem_path(file_path: &Path, cwd: &Path) -> bool {
    let anchored = if file_path.is_absolute() {
        file_path.to_path_buf()
    } else {
        cwd.join(file_path)
    };
    let resolved = resolve_dot_segments(&anchored);
    let team_dir = get_team_mem_path(cwd);
    resolved.starts_with(team_dir)
}

/// Eliminate `.` and `..` segments from an absolute path without
/// touching the filesystem. `..` at the root is a no-op — cannot
/// escape above `/`, matching POSIX + TS `path.resolve('/..')`
/// returning `/`.
///
/// For non-absolute paths the pop-on-empty behaviour would
/// silently eat leading `..` segments (differing from TS
/// `path.resolve`), so callers must absolutise first. See
/// [`is_team_mem_path`] for the anchoring pattern.
fn resolve_dot_segments(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            Component::ParentDir => {
                // Preserve root if we'd otherwise pop past it.
                let was_rooted =
                    matches!(out.components().next(), Some(Component::RootDir));
                let popped = out.pop();
                if was_rooted && !popped {
                    // We were sitting at `/` with no subpath — stay at root.
                    out = PathBuf::from("/");
                }
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

    /// Relative paths are anchored to `cwd` before resolution,
    /// matching TS `path.resolve()` which uses `process.cwd()`.
    /// Codex CR caught that the earlier implementation treated
    /// relatives as already-absolute, so any `foo/bar` would never
    /// match the team dir (absolute prefix).
    #[test]
    fn is_team_mem_path_anchors_relative_to_cwd() {
        let cwd = Path::new("/Users/alex/proj");

        // Plain relative resolves to cwd/foo.md, which is NOT
        // under the team dir (team lives under the memory base,
        // not cwd). Must return false.
        let plain_rel = Path::new("foo.md");
        assert!(!is_team_mem_path(plain_rel, cwd));

        // Relative with `..` evaluated against cwd — outside team.
        let rel_escape = Path::new("..").join("escape.md");
        assert!(!is_team_mem_path(&rel_escape, cwd));

        // Sanity: before the fix, relative paths didn't absolutise,
        // so `is_team_mem_path(Path::new("team"), cwd)` would attempt
        // to match a RELATIVE "team" against an ABSOLUTE team_dir
        // — always false. After the fix, it anchors to
        // cwd/team (still not the team dir), so still false but
        // via the correct code path. The plain-rel assertion above
        // exercises exactly that path.
    }

    /// `..` at the root is a no-op — matches POSIX and TS
    /// `path.resolve('/..')` behaviour. Earlier version would
    /// pop past root silently.
    #[test]
    fn resolve_dot_segments_cannot_escape_root() {
        let p = Path::new("/a/..").join("..");
        let resolved = resolve_dot_segments(&p);
        assert_eq!(resolved, PathBuf::from("/"));
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
