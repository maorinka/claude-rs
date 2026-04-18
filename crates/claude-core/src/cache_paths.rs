//! Cache directory paths keyed by the current working directory.
//!
//! Port of TS `src/utils/cachePaths.ts`. Every cache file (error
//! logs, message logs, MCP server logs) lives under
//! `<cache_root>/<sanitised cwd>[/…]` so concurrent sessions in
//! different projects don't overwrite each other.
//!
//! Sanitisation uses the djb2 hash, NOT the default SHA-256 so
//! cache directories stay stable across runtime upgrades — an
//! existing cache dir must not be orphaned just because we changed
//! hash functions. djb2 output is deterministic across runtimes
//! (unlike Bun's wyhash), so the on-disk layout survives a move
//! from Node to Bun or vice versa.

use crate::hash::djb2_hash;
use std::path::PathBuf;

/// Longest `[a-zA-Z0-9-]` stretch we keep verbatim. Longer names
/// get truncated + suffixed with a base36 djb2 hash of the original
/// so the full path is ≤ `MAX_SANITIZED_LENGTH + 10` chars.
pub const MAX_SANITIZED_LENGTH: usize = 200;

/// Default cache root: `~/.cache/claude-cli`. Callers that want to
/// match `env-paths('claude-cli')` exactly on Linux get this; macOS
/// differs in the TS `env-paths` impl (uses `~/Library/Caches`) —
/// callers that need that layering should compose a different root
/// via the `with_root` variants.
pub fn default_cache_root() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    home.join(".cache").join("claude-cli")
}

/// Sanitise `name` for use as a single path segment: replace every
/// char that isn't `[a-zA-Z0-9]` with `-`. If the result would
/// exceed `MAX_SANITIZED_LENGTH`, truncate and append a `-` +
/// base36(|djb2(name)|) suffix so distinct inputs stay distinct.
pub fn sanitize_path_segment(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else {
            out.push('-');
        }
    }
    if out.len() <= MAX_SANITIZED_LENGTH {
        return out;
    }
    let suffix = to_base36(djb2_hash(name).unsigned_abs() as u64);
    // Take a char-boundary-safe prefix of length MAX_SANITIZED_LENGTH.
    // `out` is ASCII by construction (alnum + '-'), so byte indexing
    // is safe.
    let prefix = &out[..MAX_SANITIZED_LENGTH];
    format!("{prefix}-{suffix}")
}

/// `<cache_root>/<project-dir>` where `<project-dir>` is
/// `sanitize_path_segment(cwd)`.
pub fn base_logs_dir(cache_root: &std::path::Path, cwd: &str) -> PathBuf {
    cache_root.join(sanitize_path_segment(cwd))
}

pub fn errors_dir(cache_root: &std::path::Path, cwd: &str) -> PathBuf {
    base_logs_dir(cache_root, cwd).join("errors")
}

pub fn messages_dir(cache_root: &std::path::Path, cwd: &str) -> PathBuf {
    base_logs_dir(cache_root, cwd).join("messages")
}

/// `<cache_root>/<project-dir>/mcp-logs-<sanitised server name>`.
/// Server name is also sanitised so Windows drive-letter colons
/// and other reserved chars don't poison the path.
pub fn mcp_logs_dir(
    cache_root: &std::path::Path,
    cwd: &str,
    server_name: &str,
) -> PathBuf {
    base_logs_dir(cache_root, cwd)
        .join(format!("mcp-logs-{}", sanitize_path_segment(server_name)))
}

fn to_base36(mut n: u64) -> String {
    if n == 0 {
        return "0".to_string();
    }
    const ALPHABET: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut buf = Vec::new();
    while n > 0 {
        buf.push(ALPHABET[(n % 36) as usize]);
        n /= 36;
    }
    buf.reverse();
    String::from_utf8(buf).expect("base36 chars are ASCII")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn sanitize_replaces_non_alnum_with_dash() {
        assert_eq!(
            sanitize_path_segment("/Users/alex/work/app"),
            "-Users-alex-work-app"
        );
    }

    #[test]
    fn sanitize_keeps_alnum_as_is() {
        assert_eq!(
            sanitize_path_segment("abc123DEF"),
            "abc123DEF"
        );
    }

    #[test]
    fn sanitize_handles_dots_and_colons() {
        // Windows drive letter + path: `C:\path\to\srv`.
        // `:` and `\` both map to `-`, so the pair `:\` becomes `--`.
        assert_eq!(
            sanitize_path_segment(r"C:\path\to\srv"),
            "C--path-to-srv"
        );
        // A simpler case with just `:` between letters.
        assert_eq!(sanitize_path_segment("server:one"), "server-one");
    }

    #[test]
    fn sanitize_truncates_when_too_long_and_suffixes_with_djb2_base36() {
        let input = "x".repeat(MAX_SANITIZED_LENGTH + 50);
        let out = sanitize_path_segment(&input);
        assert!(out.len() > MAX_SANITIZED_LENGTH);
        // Prefix is the first MAX chars.
        assert_eq!(&out[..MAX_SANITIZED_LENGTH], &"x".repeat(MAX_SANITIZED_LENGTH));
        // After the dash suffix comes the base36 djb2 hash.
        let suffix = &out[MAX_SANITIZED_LENGTH + 1..];
        let expected_suffix =
            to_base36(djb2_hash(&input).unsigned_abs() as u64);
        assert_eq!(suffix, expected_suffix);
    }

    #[test]
    fn distinct_long_inputs_produce_distinct_outputs() {
        let a = format!("{}a", "x".repeat(MAX_SANITIZED_LENGTH));
        let b = format!("{}b", "x".repeat(MAX_SANITIZED_LENGTH));
        assert_ne!(sanitize_path_segment(&a), sanitize_path_segment(&b));
    }

    #[test]
    fn base_logs_dir_joins_sanitised_cwd() {
        let root = Path::new("/cache");
        let p = base_logs_dir(root, "/Users/x");
        assert_eq!(p, PathBuf::from("/cache/-Users-x"));
    }

    #[test]
    fn errors_messages_nested_under_project_dir() {
        let root = Path::new("/cache");
        let e = errors_dir(root, "/p");
        let m = messages_dir(root, "/p");
        assert_eq!(e, PathBuf::from("/cache/-p/errors"));
        assert_eq!(m, PathBuf::from("/cache/-p/messages"));
    }

    #[test]
    fn mcp_logs_includes_server_name_prefix() {
        let root = Path::new("/cache");
        let p = mcp_logs_dir(root, "/proj", "server:one");
        assert_eq!(p, PathBuf::from("/cache/-proj/mcp-logs-server-one"));
    }

    #[test]
    fn base36_helper_zero_roundtrip() {
        assert_eq!(to_base36(0), "0");
        assert_eq!(to_base36(35), "z");
        assert_eq!(to_base36(36), "10");
    }

    #[test]
    fn default_cache_root_under_home() {
        let root = default_cache_root();
        assert!(root.ends_with(Path::new(".cache/claude-cli")));
    }
}
