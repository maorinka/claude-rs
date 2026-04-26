//! Auto-memory path resolution. Simplified port of `src/memdir/paths.ts`.
//!
//! Scope: env-var/HOME fallback only. The full TS chain (OAuth-cohort
//! `isExtractModeActive`, policy/flag/local settings override, canonical
//! git-root, Cowork override) requires the surrounding Rust config surface
//! to grow. We port the user-facing resolution so tools + prompt builders
//! have a stable answer today.

use std::path::{Path, PathBuf};

const AUTO_MEM_DIRNAME: &str = "memory";
const AUTO_MEM_ENTRYPOINT: &str = "MEMORY.md";

/// Is auto-memory enabled? Mirrors the env-var half of TS
/// `isAutoMemoryEnabled()`:
///   - `CLAUDE_CODE_DISABLE_AUTO_MEMORY` truthy → false
///   - `CLAUDE_CODE_SIMPLE` truthy → false
///   - otherwise true (settings-based opt-out not yet wired)
pub fn auto_memory_enabled() -> bool {
    fn truthy(v: &str) -> bool {
        matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    }
    if std::env::var("CLAUDE_CODE_DISABLE_AUTO_MEMORY")
        .ok()
        .as_deref()
        .map(truthy)
        .unwrap_or(false)
    {
        return false;
    }
    if std::env::var("CLAUDE_CODE_SIMPLE")
        .ok()
        .as_deref()
        .map(truthy)
        .unwrap_or(false)
    {
        return false;
    }
    true
}

/// Base directory for persistent memory storage. Mirrors TS
/// `getMemoryBaseDir()`:
///   1. `CLAUDE_CODE_REMOTE_MEMORY_DIR` env var (explicit override)
///   2. `CLAUDE_CONFIG_DIR` env var
///   3. `~/.claude`
pub fn get_memory_base_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("CLAUDE_CODE_REMOTE_MEMORY_DIR") {
        return PathBuf::from(dir);
    }
    if let Ok(dir) = std::env::var("CLAUDE_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/"))
        .join(".claude")
}

/// Derive the per-project auto-memory directory for `cwd`.
/// Shape: `{memoryBase}/projects/{sanitized-cwd}/memory/`.
pub fn get_auto_mem_path(cwd: &Path) -> PathBuf {
    let base = get_memory_base_dir();
    let sanitized = sanitize_path(cwd);
    base.join("projects").join(sanitized).join(AUTO_MEM_DIRNAME)
}

/// Full path to the MEMORY.md entrypoint for `cwd`.
pub fn get_auto_mem_entrypoint(cwd: &Path) -> PathBuf {
    get_auto_mem_path(cwd).join(AUTO_MEM_ENTRYPOINT)
}

/// Sanitize a filesystem path into something safe as a directory name.
/// Mirrors TS `sanitizePath()`: replace separators with `-` while preserving
/// the leading dash produced by an absolute path like `/Users/alice/repo`.
fn sanitize_path(p: &Path) -> PathBuf {
    let s = p.display().to_string();
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    // collapse runs of `-`
    let mut collapsed = String::with_capacity(out.len());
    let mut prev_dash = false;
    for ch in out.chars() {
        if ch == '-' {
            if !prev_dash {
                collapsed.push(ch);
            }
            prev_dash = true;
        } else {
            collapsed.push(ch);
            prev_dash = false;
        }
    }
    PathBuf::from(collapsed.trim_end_matches('-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_override_wins() {
        std::env::set_var("CLAUDE_CODE_REMOTE_MEMORY_DIR", "/tmp/remotemem");
        let base = get_memory_base_dir();
        assert_eq!(base, PathBuf::from("/tmp/remotemem"));
        std::env::remove_var("CLAUDE_CODE_REMOTE_MEMORY_DIR");
    }

    #[test]
    fn disable_env_flag_honored() {
        std::env::set_var("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "true");
        assert!(!auto_memory_enabled());
        std::env::remove_var("CLAUDE_CODE_DISABLE_AUTO_MEMORY");
    }

    #[test]
    fn path_sanitized() {
        let p = sanitize_path(Path::new("/Users/alice/work/claude-rs"));
        assert_eq!(p, PathBuf::from("-Users-alice-work-claude-rs"));
    }

    #[test]
    fn auto_mem_path_shape() {
        std::env::set_var("CLAUDE_CODE_REMOTE_MEMORY_DIR", "/tmp/mem");
        let p = get_auto_mem_path(Path::new("/Users/bob/repo"));
        assert!(p.to_string_lossy().ends_with("memory"));
        assert!(p.to_string_lossy().contains("projects"));
        std::env::remove_var("CLAUDE_CODE_REMOTE_MEMORY_DIR");
    }
}
