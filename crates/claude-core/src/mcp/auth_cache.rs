//! Disk-backed "needs auth" cache for MCP servers.
//!
//! Gap-fill ticket **G2** in the MCP client plan. Ports
//! `src/services/mcp/client.ts:257-316`.
//!
//! When a remote MCP server returns HTTP 401, the client marks it
//! as "needs-auth" and skips reconnect attempts for 15 minutes so
//! the user isn't hammered with the same re-auth prompt. This
//! state is persistent across process restarts via a small JSON
//! file at `$CLAUDE_CONFIG_HOME/mcp-needs-auth-cache.json`.
//!
//! Concurrency model
//! =================
//! - **Reads** (`is_mcp_auth_cached`) go through an in-process
//!   in-memory memo (a `RwLock<Option<Cache>>`). N concurrent
//!   reads during a batched connection share one file read.
//! - **Writes** (`set_mcp_auth_cache_entry`) serialise on a
//!   `write_lock: Mutex<()>` to prevent concurrent
//!   read-modify-write races when multiple servers 401 in the
//!   same batch. Mirrors TS's `writeChain` serialisation.
//! - After every write, the in-memory memo is invalidated so the
//!   next read picks up the new state from disk.
//!
//! Error handling mirrors TS: cache read/write failures are
//! swallowed silently (returns empty cache / logs nothing). This
//! is a best-effort optimisation, not a correctness guarantee.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::errors_util::get_claude_config_home_dir;

/// TTL for a "needs auth" entry. Matches TS `MCP_AUTH_CACHE_TTL_MS`
/// at `client.ts:257` (15 minutes in milliseconds).
pub const MCP_AUTH_CACHE_TTL_MS: u64 = 15 * 60 * 1000;

/// One entry in the cache — just a millisecond timestamp of when
/// the 401 was observed. TS shape: `{ timestamp: number }`.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct McpAuthCacheEntry {
    pub timestamp: u64,
}

/// Full cache: server-id → entry. Matches TS
/// `type McpAuthCacheData = Record<string, { timestamp: number }>`.
type McpAuthCacheData = HashMap<String, McpAuthCacheEntry>;

/// In-process memoisation of the on-disk cache. `None` means "not
/// yet read this session"; `Some(map)` is the view we're reusing
/// until a write invalidates it.
fn memo() -> &'static RwLock<Option<McpAuthCacheData>> {
    static CELL: OnceLock<RwLock<Option<McpAuthCacheData>>> = OnceLock::new();
    CELL.get_or_init(|| RwLock::new(None))
}

/// Writer serialisation. Mirrors TS's `writeChain` Promise chain:
/// concurrent `set_mcp_auth_cache_entry` calls are queued so the
/// read-modify-write cycle never overlaps.
fn write_lock() -> &'static Mutex<()> {
    static CELL: OnceLock<Mutex<()>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(()))
}

/// On-disk location: `$CLAUDE_CONFIG_HOME/mcp-needs-auth-cache.json`.
/// TS `getMcpAuthCachePath` at `client.ts:261-263`.
pub fn get_mcp_auth_cache_path() -> PathBuf {
    get_claude_config_home_dir().join("mcp-needs-auth-cache.json")
}

/// Load the cache from disk, going through the in-process memo if
/// populated. Matches TS `getMcpAuthCache` at `client.ts:271-278`.
/// A missing or malformed file yields an empty cache — best-effort
/// semantics for a feature that only affects prompt frequency, not
/// correctness.
fn load_cache() -> McpAuthCacheData {
    // Fast path: shared read of the memo.
    if let Ok(guard) = memo().read() {
        if let Some(cached) = guard.as_ref() {
            return cached.clone();
        }
    }
    // Slow path: read from disk, populate memo.
    let fresh = std::fs::read_to_string(get_mcp_auth_cache_path())
        .ok()
        .and_then(|s| serde_json::from_str::<McpAuthCacheData>(&s).ok())
        .unwrap_or_default();
    if let Ok(mut guard) = memo().write() {
        *guard = Some(fresh.clone());
    }
    fresh
}

/// Invalidate the in-memory memo so the next read re-loads from
/// disk. Called after successful writes.
fn invalidate_memo() {
    if let Ok(mut guard) = memo().write() {
        *guard = None;
    }
}

/// Current timestamp in milliseconds since the UNIX epoch.
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Has this server been marked "needs auth" within the last
/// 15 minutes? Matches TS `isMcpAuthCached` at `client.ts:280-287`.
///
/// Returns `false` when:
/// - No entry for this server.
/// - Entry exists but is older than `MCP_AUTH_CACHE_TTL_MS`.
/// - Cache file is missing / unreadable / malformed.
pub fn is_mcp_auth_cached(server_id: &str) -> bool {
    let cache = load_cache();
    match cache.get(server_id) {
        Some(entry) => now_ms().saturating_sub(entry.timestamp) < MCP_AUTH_CACHE_TTL_MS,
        None => false,
    }
}

/// Record that this server needs re-auth, setting the entry's
/// timestamp to `now`. Serialises against concurrent writes via
/// the internal `write_lock`. Matches TS
/// `setMcpAuthCacheEntry` at `client.ts:293-309`.
///
/// Any I/O error (path unwritable, parent dir can't be created,
/// JSON serialise fails) is swallowed silently — TS catches too.
pub fn set_mcp_auth_cache_entry(server_id: &str) {
    let _guard = match write_lock().lock() {
        Ok(g) => g,
        Err(_) => return, // Poisoned lock — best-effort, bail.
    };
    let mut cache = load_cache();
    cache.insert(
        server_id.to_string(),
        McpAuthCacheEntry { timestamp: now_ms() },
    );
    let path = get_mcp_auth_cache_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(body) = serde_json::to_string(&cache) {
        let _ = std::fs::write(&path, body);
    }
    invalidate_memo();
}

/// Wipe the cache: invalidate the memo and remove the on-disk
/// file (if it exists). Matches TS `clearMcpAuthCache` at
/// `client.ts:311-316`.
pub fn clear_mcp_auth_cache() {
    invalidate_memo();
    let _ = std::fs::remove_file(get_mcp_auth_cache_path());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Tests mutate CLAUDE_CONFIG_DIR + the on-disk file + the
    // static memo — serialise across tests to keep them isolated.
    static T_LOCK: Mutex<()> = Mutex::new(());

    /// Scoped override of CLAUDE_CONFIG_DIR that rewinds on drop.
    struct ConfigHomeGuard {
        prev: Option<std::ffi::OsString>,
    }

    impl ConfigHomeGuard {
        fn set(path: &std::path::Path) -> Self {
            let prev = std::env::var_os("CLAUDE_CONFIG_DIR");
            std::env::set_var("CLAUDE_CONFIG_DIR", path);
            invalidate_memo();
            Self { prev }
        }
    }

    impl Drop for ConfigHomeGuard {
        fn drop(&mut self) {
            match &self.prev {
                Some(v) => std::env::set_var("CLAUDE_CONFIG_DIR", v),
                None => std::env::remove_var("CLAUDE_CONFIG_DIR"),
            }
            invalidate_memo();
        }
    }

    fn setup_tmp_home() -> (tempfile::TempDir, ConfigHomeGuard) {
        let tmp = tempfile::tempdir().unwrap();
        let guard = ConfigHomeGuard::set(tmp.path());
        (tmp, guard)
    }

    #[test]
    fn missing_cache_file_reports_not_cached() {
        let _g = T_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let (_tmp, _guard) = setup_tmp_home();
        clear_mcp_auth_cache();
        assert!(!is_mcp_auth_cached("server-a"));
    }

    #[test]
    fn set_then_check_reports_cached() {
        let _g = T_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let (_tmp, _guard) = setup_tmp_home();
        clear_mcp_auth_cache();
        set_mcp_auth_cache_entry("server-b");
        assert!(is_mcp_auth_cached("server-b"));
        // Unrelated server still reads false.
        assert!(!is_mcp_auth_cached("server-c"));
    }

    #[test]
    fn expired_entry_reports_not_cached() {
        let _g = T_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let (_tmp, _guard) = setup_tmp_home();
        clear_mcp_auth_cache();

        // Hand-write an entry with a timestamp older than the TTL.
        let path = get_mcp_auth_cache_path();
        let stale_ms = now_ms().saturating_sub(MCP_AUTH_CACHE_TTL_MS + 1_000);
        let body = format!(r#"{{"stale":{{"timestamp":{}}}}}"#, stale_ms);
        std::fs::write(&path, body).unwrap();
        invalidate_memo();

        assert!(!is_mcp_auth_cached("stale"));
    }

    #[test]
    fn set_overwrites_existing_entry_with_fresh_timestamp() {
        let _g = T_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let (_tmp, _guard) = setup_tmp_home();
        clear_mcp_auth_cache();

        // Seed a stale entry directly on disk.
        let path = get_mcp_auth_cache_path();
        let stale_ms = now_ms().saturating_sub(MCP_AUTH_CACHE_TTL_MS + 1_000);
        let body = format!(r#"{{"svc":{{"timestamp":{}}}}}"#, stale_ms);
        std::fs::write(&path, body).unwrap();
        invalidate_memo();
        assert!(!is_mcp_auth_cached("svc"));

        // Re-setting should refresh the timestamp → now cached.
        set_mcp_auth_cache_entry("svc");
        assert!(is_mcp_auth_cached("svc"));
    }

    #[test]
    fn clear_removes_cache_file_and_invalidates_memo() {
        let _g = T_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let (_tmp, _guard) = setup_tmp_home();
        set_mcp_auth_cache_entry("svc");
        let path = get_mcp_auth_cache_path();
        assert!(path.exists(), "cache file should exist after set");
        assert!(is_mcp_auth_cached("svc"));

        clear_mcp_auth_cache();
        assert!(!path.exists(), "clear should remove the cache file");
        assert!(!is_mcp_auth_cached("svc"), "memo must be invalidated too");
    }

    #[test]
    fn malformed_cache_file_is_treated_as_empty() {
        let _g = T_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let (_tmp, _guard) = setup_tmp_home();
        let path = get_mcp_auth_cache_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, "NOT JSON").unwrap();
        invalidate_memo();
        // Malformed → best-effort empty — no panic, reports false.
        assert!(!is_mcp_auth_cached("anything"));
    }

    #[test]
    fn memo_is_reused_across_reads() {
        let _g = T_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let (_tmp, _guard) = setup_tmp_home();
        clear_mcp_auth_cache();
        set_mcp_auth_cache_entry("svc");

        // After set, memo is invalidated; next read re-loads.
        assert!(is_mcp_auth_cached("svc"));
        // Now delete the file out-of-band; the memo should still
        // answer the question without re-reading (until
        // `invalidate_memo` or a write happens).
        std::fs::remove_file(get_mcp_auth_cache_path()).unwrap();
        assert!(
            is_mcp_auth_cached("svc"),
            "memo must answer without re-reading from disk"
        );
    }
}
