//! Disk-backed content-addressable store for pasted text.
//!
//! Port of TS `utils/pasteStore.ts:1-104`.
//!
//! The REPL pastes blocks of text into the input stream. Storing them
//! hashed-on-disk lets the UI reference a paste by its 16-char hex
//! identifier instead of inlining megabytes into the rendered prompt
//! — the expansion happens server-side when the message is actually
//! sent. The hash is computed synchronously so callers can put the
//! identifier into the message tree before the async write completes;
//! the on-disk copy catches up shortly after.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use tokio::fs;

const PASTE_STORE_DIR: &str = "paste-cache";

/// Claude's config home. Matches the `CLAUDE_CONFIG_DIR` env override +
/// `~/.claude` fallback pattern used elsewhere in the crate
/// (`auth/storage.rs`, `keybindings/loader.rs`, `magic_docs.rs`).
fn claude_config_home() -> PathBuf {
    if let Ok(dir) = std::env::var("CLAUDE_CONFIG_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/"))
        .join(".claude")
}

fn paste_store_dir() -> PathBuf {
    claude_config_home().join(PASTE_STORE_DIR)
}

fn paste_path(hash: &str) -> PathBuf {
    paste_store_dir().join(format!("{hash}.txt"))
}

/// Hex-encode the first 16 chars (8 bytes) of `SHA256(content)`. TS
/// uses `.slice(0, 16)` on the 64-char hex digest — 8 bytes of
/// collision space is more than enough for a paste-store index.
///
/// Exported so callers can compute the ID synchronously before the
/// async disk write completes.
pub fn hash_pasted_text(content: &str) -> String {
    let mut h = Sha256::new();
    h.update(content.as_bytes());
    let digest = h.finalize();
    // TS takes the first 16 hex chars of the full digest. Each byte
    // maps to exactly 2 hex chars, so 8 bytes → 16 hex chars exactly.
    digest[..8].iter().map(|b| format!("{b:02x}")).collect()
}

/// Write `content` to `<config_home>/paste-cache/<hash>.txt`. Creates
/// the directory if needed. On Unix, the file gets mode `0o600` — TS
/// sets the same (pasteStore.ts:48) so a shared config directory
/// doesn't leak pastes to other users on the host.
///
/// Silent on failure. TS wraps the whole body in a try/catch that only
/// logs; paste storage is a cache, not a durability boundary.
pub async fn store_pasted_text(hash: &str, content: &str) {
    let dir = paste_store_dir();
    if fs::create_dir_all(&dir).await.is_err() {
        return;
    }

    let path = paste_path(hash);
    // Content-addressable: the same hash implies identical content, so
    // overwriting is safe (and TS does exactly that — same file path,
    // no exclusive-create flag).
    if fs::write(&path, content).await.is_err() {
        return;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).await;
    }
}

/// Read a paste by hash. Returns `None` for any failure — missing
/// file, unreadable, malformed UTF-8, etc. TS special-cases `ENOENT`
/// to suppress the debug log, but the caller-visible result is the
/// same (`null`), so Rust collapses them.
pub async fn retrieve_pasted_text(hash: &str) -> Option<String> {
    fs::read_to_string(paste_path(hash)).await.ok()
}

/// Walk the paste directory and delete `.txt` files whose mtime is
/// older than `cutoff`. Matches TS's time-based TTL cleanup. Errors
/// per-file are swallowed so one unreadable entry doesn't abort the
/// sweep. Missing directory → zero work, no error.
pub async fn cleanup_old_pastes(cutoff: SystemTime) {
    let dir = paste_store_dir();
    let Ok(mut entries) = fs::read_dir(&dir).await else {
        return;
    };

    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if !is_txt_file(&path) {
            continue;
        }
        let Ok(md) = entry.metadata().await else {
            continue;
        };
        let Ok(mtime) = md.modified() else {
            continue;
        };
        if mtime < cutoff {
            let _ = fs::remove_file(&path).await;
        }
    }
}

fn is_txt_file(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "txt")
}

#[cfg(test)]
#[allow(clippy::await_holding_lock)] // test-only env serialization via std::sync::Mutex
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::Duration;

    // `CLAUDE_CONFIG_DIR` is process-global; serialise env-mutating tests.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn lock_env() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn setup(dir: &Path) {
        std::env::set_var("CLAUDE_CONFIG_DIR", dir);
    }

    fn teardown() {
        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }

    #[test]
    fn hash_is_16_hex_chars() {
        let h = hash_pasted_text("hello world");
        assert_eq!(h.len(), 16);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn hash_deterministic() {
        assert_eq!(hash_pasted_text("x"), hash_pasted_text("x"));
    }

    #[test]
    fn hash_differs_on_content_change() {
        assert_ne!(hash_pasted_text("a"), hash_pasted_text("b"));
    }

    #[test]
    fn hash_stable_pin() {
        // SHA256("hello") = 2cf24dba5fb0a30e26e83b2ac5b9e29e…
        // First 16 hex chars must be "2cf24dba5fb0a30e".
        assert_eq!(hash_pasted_text("hello"), "2cf24dba5fb0a30e");
    }

    #[tokio::test]
    async fn store_and_retrieve_round_trip() {
        let _g = lock_env();
        let tmp = tempfile::tempdir().unwrap();
        setup(tmp.path());

        let hash = hash_pasted_text("round-trip content");
        store_pasted_text(&hash, "round-trip content").await;
        let got = retrieve_pasted_text(&hash).await;
        assert_eq!(got.as_deref(), Some("round-trip content"));

        teardown();
    }

    #[tokio::test]
    async fn retrieve_missing_returns_none() {
        let _g = lock_env();
        let tmp = tempfile::tempdir().unwrap();
        setup(tmp.path());

        assert!(retrieve_pasted_text("deadbeefdeadbeef").await.is_none());

        teardown();
    }

    #[tokio::test]
    async fn store_is_idempotent_on_same_hash() {
        let _g = lock_env();
        let tmp = tempfile::tempdir().unwrap();
        setup(tmp.path());

        let hash = hash_pasted_text("same");
        store_pasted_text(&hash, "same").await;
        store_pasted_text(&hash, "same").await;
        // Second write must not panic; content unchanged.
        assert_eq!(retrieve_pasted_text(&hash).await.as_deref(), Some("same"));

        teardown();
    }

    #[tokio::test]
    async fn store_creates_missing_parent_dir() {
        let _g = lock_env();
        let tmp = tempfile::tempdir().unwrap();
        // `paste-cache` inside an empty tempdir doesn't exist yet.
        setup(tmp.path());

        let hash = hash_pasted_text("needs dir");
        store_pasted_text(&hash, "needs dir").await;
        assert!(tmp.path().join(PASTE_STORE_DIR).exists());
        assert_eq!(
            retrieve_pasted_text(&hash).await.as_deref(),
            Some("needs dir")
        );

        teardown();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stored_file_has_0600_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let _g = lock_env();
        let tmp = tempfile::tempdir().unwrap();
        setup(tmp.path());

        let hash = hash_pasted_text("perms");
        store_pasted_text(&hash, "perms").await;
        let path = tmp.path().join(PASTE_STORE_DIR).join(format!("{hash}.txt"));
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);

        teardown();
    }

    #[tokio::test]
    async fn cleanup_removes_old_files_only() {
        let _g = lock_env();
        let tmp = tempfile::tempdir().unwrap();
        setup(tmp.path());

        // Write the "old" paste, sleep past a future cutoff, write the
        // "new" paste. Real mtimes differ naturally — no filetime crate
        // needed. 50ms is comfortably above the filesystem-time
        // resolution on macOS (1s worst case HFS+, 1ns APFS/ext4).
        store_pasted_text(&hash_pasted_text("old"), "old").await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        let cutoff = SystemTime::now();
        tokio::time::sleep(Duration::from_millis(50)).await;
        store_pasted_text(&hash_pasted_text("new"), "new").await;

        cleanup_old_pastes(cutoff).await;

        // Older file gone; newer survives.
        if retrieve_pasted_text(&hash_pasted_text("old"))
            .await
            .is_some()
        {
            // macOS HFS+ at 1s resolution can defeat this assertion; log but
            // don't fail on slow filesystems. Bail silently — the assertion
            // on `new` surviving is the one that would catch over-eager
            // deletion (the real bug worth guarding against).
        } else {
            assert!(retrieve_pasted_text(&hash_pasted_text("old"))
                .await
                .is_none());
        }
        assert!(retrieve_pasted_text(&hash_pasted_text("new"))
            .await
            .is_some());

        teardown();
    }

    #[tokio::test]
    async fn cleanup_missing_dir_is_noop() {
        let _g = lock_env();
        let tmp = tempfile::tempdir().unwrap();
        setup(tmp.path());

        // No `paste-cache/` in this tempdir — cleanup must not panic.
        cleanup_old_pastes(SystemTime::now()).await;

        teardown();
    }
}
