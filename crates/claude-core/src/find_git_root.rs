//! Walk up the directory tree to locate a git repository root.
//!
//! Port of TS `utils/git.ts:25-109` (`findGitRoot` + `findGitRootImpl`).
//!
//! Returns the directory containing `.git` (as a directory OR a file —
//! worktrees and submodules use a `.git` file that points at the real
//! gitdir), or `None` if no git root exists between `start_path` and
//! the filesystem root.
//!
//! The TS version memoises results with an LRU(50) cache because
//! `gitDiff` invokes `findGitRoot(dirname(file))` per edit, so a
//! many-file session would otherwise accumulate entries forever.
//! The Rust port **does not memoise** — `std::fs::metadata` is cheap
//! on modern filesystems (kernel cache makes repeat `.git` lookups
//! effectively free), and forcing callers to reason about an invisible
//! cache would hide the real cost. Callers that need caching can wrap
//! this function at their own level.
//!
//! Diagnostic events (`find_git_root_started` / `_completed`) are
//! emitted via the already-ported [`diag_logs`] module so the same
//! telemetry stream picks them up.
//!
//! [`diag_logs`]: crate::diag_logs

use crate::diag_logs::{log_for_diagnostics_no_pii, DiagnosticLogLevel};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::time::Instant;

fn has_git_marker(dir: &Path) -> bool {
    // `.git` may be a directory (normal repo), a regular file (worktree /
    // submodule pointer), or — per the TS code's `isDirectory() ||
    // isFile()` check — explicitly NOT a symlink-to-nothing or device
    // node. Rust's `Metadata::file_type()` gives us the same test.
    let git_path = dir.join(".git");
    match std::fs::metadata(&git_path) {
        Ok(md) => {
            let ft = md.file_type();
            ft.is_dir() || ft.is_file()
        }
        Err(_) => false,
    }
}

/// Walk upward from `start_path` looking for a `.git` marker. Returns
/// the directory that contains it, or `None`.
///
/// The TS port normalises the result with `.normalize('NFC')` to canonicalise
/// Unicode composition — a real concern on macOS HFS+ where filesystem
/// entries are NFD. Rust's `PathBuf` preserves whatever bytes the
/// filesystem returns; callers that compare paths across APIs that may
/// normalise differently should do their own NFC pass (the
/// `unicode-normalization` crate is already a dep).
pub fn find_git_root(start_path: &Path) -> Option<PathBuf> {
    let start = Instant::now();
    log_for_diagnostics_no_pii(DiagnosticLogLevel::Info, "find_git_root_started", None);

    let canonical = start_path
        .canonicalize()
        .unwrap_or_else(|_| start_path.to_path_buf());

    let mut current = canonical.as_path();
    let mut stat_count: u64 = 0;

    loop {
        stat_count += 1;
        if has_git_marker(current) {
            log_completion(start, stat_count, true);
            return Some(current.to_path_buf());
        }

        match current.parent() {
            // Walked off the top of the tree; TS checks root explicitly
            // in a separate block, but `Path::parent()` on `/` returns
            // `None`, so the check merges with the loop's terminator.
            None => {
                log_completion(start, stat_count, false);
                return None;
            }
            Some(parent) if parent == current => {
                // Shouldn't happen in modern Rust — parent of root is
                // None — but keep the guard for exotic filesystems.
                log_completion(start, stat_count, false);
                return None;
            }
            Some(parent) => {
                current = parent;
            }
        }
    }
}

fn log_completion(start: Instant, stat_count: u64, found: bool) {
    let mut data = Map::new();
    data.insert(
        "duration_ms".into(),
        Value::from(start.elapsed().as_millis() as u64),
    );
    data.insert("stat_count".into(), Value::from(stat_count));
    data.insert("found".into(), Value::from(found));
    log_for_diagnostics_no_pii(
        DiagnosticLogLevel::Info,
        "find_git_root_completed",
        Some(data),
    );
}

/// `find_git_root(cwd).is_some()`. TS `memory/versions.ts:6-7`
/// (`projectIsInGitRepo`).
pub fn project_is_in_git_repo(cwd: &Path) -> bool {
    find_git_root(cwd).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn finds_repo_at_start_path() {
        let dir = tmp();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        assert_eq!(
            find_git_root(dir.path()).unwrap().canonicalize().unwrap(),
            dir.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn finds_repo_by_walking_up() {
        let root = tmp();
        std::fs::create_dir(root.path().join(".git")).unwrap();
        let nested = root.path().join("a/b/c");
        std::fs::create_dir_all(&nested).unwrap();

        assert_eq!(
            find_git_root(&nested).unwrap().canonicalize().unwrap(),
            root.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn finds_repo_via_dot_git_file() {
        // Worktrees and submodules use a regular file at `.git` that
        // points at the real gitdir. The predicate must accept both.
        let dir = tmp();
        std::fs::write(dir.path().join(".git"), b"gitdir: /some/path").unwrap();

        assert_eq!(
            find_git_root(dir.path()).unwrap().canonicalize().unwrap(),
            dir.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn returns_none_when_no_git_above() {
        // Build a nested tree with no `.git` anywhere inside. We can't
        // guarantee the host filesystem has no `.git` above the tempdir
        // (e.g. running from a repo checkout), so assert equivalence
        // with a direct check rather than expecting `None`.
        let dir = tmp();
        let nested = dir.path().join("a/b");
        std::fs::create_dir_all(&nested).unwrap();

        let nested_result = find_git_root(&nested);
        let dir_result = find_git_root(dir.path());
        // Whatever the ambient state, deep and shallow lookups agree:
        // they either both hit the same ancestor repo or both return None.
        match (nested_result, dir_result) {
            (Some(a), Some(b)) => assert_eq!(a.canonicalize().unwrap(), b.canonicalize().unwrap()),
            (None, None) => {}
            (a, b) => panic!("asymmetric: nested={a:?} dir={b:?}"),
        }
    }

    #[test]
    fn project_is_in_git_repo_matches_find_git_root() {
        let dir = tmp();
        assert!(!project_is_in_git_repo(dir.path()) || find_git_root(dir.path()).is_some());

        std::fs::create_dir(dir.path().join(".git")).unwrap();
        assert!(project_is_in_git_repo(dir.path()));
    }

    #[test]
    fn non_existent_start_path_does_not_panic() {
        // `canonicalize` fails for a missing path; the function falls back
        // to the raw path and walks up from there. Must not panic.
        let fake = Path::new("/definitely/does/not/exist/anywhere");
        let _ = find_git_root(fake);
    }

    #[test]
    fn nested_git_root_wins_over_parent() {
        // If both a parent AND a child have `.git`, the child is closer
        // to `start_path` and must be returned — the walk goes bottom-up.
        let outer = tmp();
        std::fs::create_dir(outer.path().join(".git")).unwrap();
        let inner = outer.path().join("submodule");
        std::fs::create_dir(&inner).unwrap();
        std::fs::write(inner.join(".git"), b"gitdir: /elsewhere").unwrap();

        let deep = inner.join("src");
        std::fs::create_dir(&deep).unwrap();

        assert_eq!(
            find_git_root(&deep).unwrap().canonicalize().unwrap(),
            inner.canonicalize().unwrap()
        );
    }
}
