//! Path-expansion + safety helpers.
//!
//! Port of the Rust-portable pieces of `src/utils/path.ts`. The TS
//! version delegates to `posixPathToWindowsPath` + `getFsImplementation`
//! for cross-platform resolution; std::path on Rust covers the same
//! ground natively. Exported helpers:
//!   - `expand_path(path, base_dir)` — handles `~`, `~/relative`,
//!     absolute, relative, with null-byte + empty rejection.
//!   - `to_relative_path(abs, cwd)` — converts to cwd-relative if the
//!     relative form doesn't escape cwd; otherwise keeps absolute.
//!   - `contains_path_traversal(path)` — catches `..` segments.
//!   - `sanitize_path(path)` — stable, collision-proof slug for using
//!     a filesystem path as a directory name.
//!   - `normalize_path_for_config_key(path)` — forward-slash
//!     canonicalisation for JSON config keys.

use std::path::{Component, Path, PathBuf};

/// Expand a path that may contain `~` / `~/...` notation to an
/// absolute path, resolving relative inputs against `base_dir`.
///
/// Returns an error on null bytes or unsupported input.
pub fn expand_path(path: &str, base_dir: Option<&Path>) -> Result<PathBuf, PathError> {
    if path.contains('\0') {
        return Err(PathError::NullByte);
    }
    if let Some(bd) = base_dir {
        if bd.as_os_str().as_encoded_bytes().contains(&0) {
            return Err(PathError::NullByte);
        }
    }

    let trimmed = path.trim();

    let base = match base_dir {
        Some(b) => b.to_path_buf(),
        None => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    };

    if trimmed.is_empty() {
        return Ok(normalize_path(&base));
    }

    if trimmed == "~" {
        return Ok(home_dir_fallback());
    }

    if let Some(rest) = trimmed.strip_prefix("~/") {
        let home = home_dir_fallback();
        return Ok(home.join(rest));
    }

    let p = Path::new(trimmed);
    if p.is_absolute() {
        return Ok(normalize_path(p));
    }

    Ok(normalize_path(&base.join(p)))
}

fn home_dir_fallback() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

/// Resolve `.` and `..` components without touching the filesystem.
/// Stdlib's `Path::canonicalize` requires the path to exist; this
/// pure-syntactic version is safer for user input + tests.
pub fn normalize_path(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !matches!(out.components().next_back(), Some(Component::RootDir))
                    && !out.as_os_str().is_empty()
                {
                    out.pop();
                }
            }
            other => out.push(other),
        }
    }
    if out.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        out
    }
}

/// Convert an absolute path to a cwd-relative form — iff the relative
/// form doesn't start with `..`. Saves tokens in tool output by
/// rendering `src/main.rs` instead of `/home/alice/proj/src/main.rs`.
/// Matches TS `toRelativePath`.
pub fn to_relative_path(absolute: &Path, cwd: &Path) -> PathBuf {
    let Ok(rel) = absolute.strip_prefix(cwd) else {
        return absolute.to_path_buf();
    };
    let rel_buf = rel.to_path_buf();
    // If the relative form starts with .. (should be impossible via
    // strip_prefix, but be defensive) keep absolute.
    if rel_buf
        .components()
        .next()
        .is_some_and(|c| matches!(c, Component::ParentDir))
    {
        absolute.to_path_buf()
    } else {
        rel_buf
    }
}

/// True iff the path contains a `..` traversal segment.
pub fn contains_path_traversal(path: &str) -> bool {
    for comp in path.split(['/', '\\']) {
        if comp == ".." {
            return true;
        }
    }
    false
}

/// Replace non-alphanumeric + non-dash + non-underscore chars with `-`,
/// strip leading/trailing `-`, and collapse runs. Produces a stable
/// directory-name slug for a filesystem path. Matches the intent of
/// TS `sanitizePath` without the cross-platform quirks (TS handles
/// Windows drive letters via posixPathToWindowsPath; we accept
/// whatever the caller passes).
pub fn sanitize_path(path: &str) -> String {
    let trimmed = path.trim_start_matches('/').trim_start_matches('\\');
    let mut collapsed = String::with_capacity(trimmed.len());
    let mut prev_dash = false;
    for ch in trimmed.chars() {
        let keep = ch.is_ascii_alphanumeric() || ch == '-' || ch == '_';
        if keep {
            collapsed.push(ch);
            prev_dash = false;
        } else if !prev_dash {
            collapsed.push('-');
            prev_dash = true;
        }
    }
    collapsed.trim_matches('-').to_string()
}

/// Normalize a path for use as a JSON config key. Forward-slashes
/// only — works on both POSIX and Windows. Matches TS
/// `normalizePathForConfigKey`.
pub fn normalize_path_for_config_key(path: &str) -> String {
    let normalized = normalize_path(Path::new(path));
    normalized.to_string_lossy().replace('\\', "/")
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PathError {
    #[error("path contains null bytes")]
    NullByte,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_empty_returns_base_dir_normalized() {
        let base = PathBuf::from("/home/user");
        let p = expand_path("", Some(&base)).unwrap();
        assert_eq!(p, PathBuf::from("/home/user"));
    }

    #[test]
    fn expand_tilde_returns_home() {
        let p = expand_path("~", None).unwrap();
        assert_eq!(p, home_dir_fallback());
    }

    #[test]
    fn expand_tilde_slash_joins_home() {
        let p = expand_path("~/docs", None).unwrap();
        assert_eq!(p, home_dir_fallback().join("docs"));
    }

    #[test]
    fn expand_absolute_passthrough() {
        let base = PathBuf::from("/home/user");
        let p = expand_path("/etc/hosts", Some(&base)).unwrap();
        assert_eq!(p, PathBuf::from("/etc/hosts"));
    }

    #[test]
    fn expand_relative_resolves_against_base() {
        let base = PathBuf::from("/proj");
        let p = expand_path("./src", Some(&base)).unwrap();
        assert_eq!(p, PathBuf::from("/proj/src"));
    }

    #[test]
    fn expand_rejects_null_byte() {
        let err = expand_path("/tmp/\0x", None).unwrap_err();
        assert_eq!(err, PathError::NullByte);
    }

    #[test]
    fn normalize_resolves_dot_and_dotdot() {
        assert_eq!(
            normalize_path(Path::new("/a/./b/../c")),
            PathBuf::from("/a/c")
        );
    }

    #[test]
    fn normalize_empty_becomes_dot() {
        assert_eq!(normalize_path(Path::new("")), PathBuf::from("."));
    }

    #[test]
    fn relative_inside_cwd_returns_relative() {
        let cwd = PathBuf::from("/proj");
        let abs = PathBuf::from("/proj/src/main.rs");
        let rel = to_relative_path(&abs, &cwd);
        assert_eq!(rel, PathBuf::from("src/main.rs"));
    }

    #[test]
    fn relative_outside_cwd_keeps_absolute() {
        let cwd = PathBuf::from("/proj");
        let abs = PathBuf::from("/etc/hosts");
        let rel = to_relative_path(&abs, &cwd);
        assert_eq!(rel, PathBuf::from("/etc/hosts"));
    }

    #[test]
    fn traversal_detection() {
        assert!(contains_path_traversal("../foo"));
        assert!(contains_path_traversal("foo/../bar"));
        assert!(contains_path_traversal("foo\\..\\bar"));
        assert!(!contains_path_traversal("foo/bar"));
        assert!(!contains_path_traversal("foo.bar"));
        // Leading dots inside a filename shouldn't trigger.
        assert!(!contains_path_traversal(".hidden"));
        assert!(!contains_path_traversal("foo/.hidden"));
    }

    #[test]
    fn sanitize_produces_stable_slug() {
        assert_eq!(
            sanitize_path("/Users/alice/code/claude-rs"),
            "Users-alice-code-claude-rs"
        );
        assert_eq!(sanitize_path("C:\\Users\\bob\\proj"), "C-Users-bob-proj");
        assert_eq!(sanitize_path("///leading"), "leading");
    }

    #[test]
    fn sanitize_collapses_runs_of_dashes() {
        assert_eq!(sanitize_path("foo//bar//baz"), "foo-bar-baz");
    }

    #[test]
    fn sanitize_strips_trailing_dashes() {
        assert_eq!(sanitize_path("foo///"), "foo");
    }

    #[test]
    fn normalize_path_for_config_key_converts_backslashes() {
        assert_eq!(normalize_path_for_config_key("C:\\foo\\bar"), "C:/foo/bar");
        assert_eq!(normalize_path_for_config_key("/a/./b/../c"), "/a/c");
    }
}
