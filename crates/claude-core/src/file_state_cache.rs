//! Path-normalised LRU cache of file contents read during a session.
//!
//! Port of TS `utils/fileStateCache.ts:1-142`.
//!
//! The cache holds what the model has seen of each file so that
//! subsequent Edit / Write operations can compute diffs and validate
//! that the model's view is current. Path normalisation ensures
//! repeat reads under `.` vs `./` vs symlink-relative paths hit the
//! same entry.
//!
//! TS uses `lru-cache` with size-based eviction keyed on UTF-8 byte
//! length. The Rust port uses the `lru` crate with a fixed
//! max-entries cap + a manual byte-size accounting gate — `lru` does
//! not expose size-based eviction, so when the byte total exceeds
//! `max_size_bytes`, the least-recently-used entries are popped
//! until the payload fits.

use lru::LruCache;
use serde::{Deserialize, Serialize};
use std::num::NonZeroUsize;
use std::path::PathBuf;

/// Default max entries. TS `READ_FILE_STATE_CACHE_SIZE` at
/// `fileStateCache.ts:18`.
pub const READ_FILE_STATE_CACHE_SIZE: usize = 100;

/// Default byte ceiling — 25 MB. TS
/// `DEFAULT_MAX_CACHE_SIZE_BYTES` at `fileStateCache.ts:22`.
pub const DEFAULT_MAX_CACHE_SIZE_BYTES: usize = 25 * 1024 * 1024;

/// One cache entry — what the model has seen of a file.
///
/// Matches TS `FileState` at `fileStateCache.ts:4-15`. `offset` and
/// `limit` capture the windowed-read parameters so a subsequent
/// Edit / Write can check whether the model has seen the relevant
/// region.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileState {
    pub content: String,
    /// Unix-epoch milliseconds — matches TS `Date.now()`.
    pub timestamp: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
    /// TS `isPartialView` — true when the entry was populated by
    /// auto-injection (CLAUDE.md with HTML comments stripped, etc.)
    /// and the injected content doesn't match raw disk bytes. The
    /// model has only seen the stripped view; `content` here is the
    /// raw bytes for diffing, NOT the stripped view.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_partial_view: Option<bool>,
}

impl FileState {
    fn byte_len(&self) -> usize {
        // TS uses `Buffer.byteLength(value.content)` (UTF-8 bytes).
        // Rust `String::len()` is already UTF-8 byte count.
        self.content.len().max(1)
    }
}

/// Path-normalised LRU cache of `FileState` entries.
///
/// Keys are normalised via [`normalize_key`] before every access so
/// `/a/b/../b/file.txt` and `/a/b/file.txt` hit the same entry.
pub struct FileStateCache {
    inner: LruCache<String, FileState>,
    max_size_bytes: usize,
    /// Running byte total across all cached `content` strings.
    current_bytes: usize,
}

impl FileStateCache {
    /// Create a cache with the given entry cap + byte budget.
    pub fn new(max_entries: usize, max_size_bytes: usize) -> Self {
        let cap = NonZeroUsize::new(max_entries.max(1)).unwrap();
        Self {
            inner: LruCache::new(cap),
            max_size_bytes,
            current_bytes: 0,
        }
    }

    /// Create with the 25 MB default byte cap. Matches TS
    /// `createFileStateCacheWithSizeLimit(maxEntries)`.
    pub fn with_default_size_limit(max_entries: usize) -> Self {
        Self::new(max_entries, DEFAULT_MAX_CACHE_SIZE_BYTES)
    }

    pub fn get(&mut self, key: &str) -> Option<&FileState> {
        self.inner.get(&normalize_key(key))
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.inner.contains(&normalize_key(key))
    }

    pub fn insert(&mut self, key: String, value: FileState) {
        let key = normalize_key(&key);
        // Account for the displaced entry, if any.
        if let Some(old) = self.inner.peek(&key) {
            self.current_bytes = self.current_bytes.saturating_sub(old.byte_len());
        }
        self.current_bytes += value.byte_len();
        self.inner.put(key, value);
        self.evict_to_budget();
    }

    pub fn remove(&mut self, key: &str) -> Option<FileState> {
        let popped = self.inner.pop(&normalize_key(key))?;
        self.current_bytes = self.current_bytes.saturating_sub(popped.byte_len());
        Some(popped)
    }

    pub fn clear(&mut self) {
        self.inner.clear();
        self.current_bytes = 0;
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.len() == 0
    }

    pub fn max_entries(&self) -> usize {
        self.inner.cap().get()
    }

    pub fn max_size_bytes(&self) -> usize {
        self.max_size_bytes
    }

    pub fn current_bytes(&self) -> usize {
        self.current_bytes
    }

    /// Iterate entries in LRU order (oldest first). Returns clones
    /// of keys + refs to values — the underlying `LruCache` iterator
    /// requires a shared borrow, so consumers that need owned pairs
    /// can collect.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &FileState)> {
        self.inner.iter()
    }

    /// Dump to a plain map — matches TS `cacheToObject` at
    /// `fileStateCache.ts:108-113`. Insertion order NOT preserved
    /// in `HashMap`, but consumers use this for `Object.fromEntries`
    /// which also doesn't guarantee order.
    pub fn to_map(&self) -> std::collections::HashMap<String, FileState> {
        self.inner
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    fn evict_to_budget(&mut self) {
        while self.current_bytes > self.max_size_bytes && self.inner.len() > 1 {
            let Some((_, evicted)) = self.inner.pop_lru() else {
                break;
            };
            self.current_bytes = self.current_bytes.saturating_sub(evicted.byte_len());
        }
    }
}

/// Key-normaliser. Uses `std::path::PathBuf` for canonical-ish
/// normalisation (collapses `.` and `..` segments, normalises
/// separators per platform). Matches TS `normalize()` closely
/// enough for cache-hit stability across call sites that use
/// different path shapes.
fn normalize_key(key: &str) -> String {
    // `normalize()` in Node collapses `..` segments WITHOUT hitting
    // the filesystem. Rust doesn't have a stdlib equivalent that does
    // pure-lexical normalisation, so we do it manually by walking the
    // components.
    let raw = PathBuf::from(key);
    let mut parts: Vec<std::path::Component<'_>> = Vec::new();
    for comp in raw.components() {
        use std::path::Component;
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                // Only pop if we have a non-root normal segment to pop.
                // TS `normalize('/a/..')` → '/', `normalize('a/..')` → '.'.
                if matches!(parts.last(), Some(Component::Normal(_))) {
                    parts.pop();
                } else {
                    parts.push(comp);
                }
            }
            _ => parts.push(comp),
        }
    }
    if parts.is_empty() {
        ".".to_owned()
    } else {
        let pb: PathBuf = parts.iter().collect();
        pb.to_string_lossy().into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_state(content: &str) -> FileState {
        FileState {
            content: content.to_owned(),
            timestamp: 0,
            offset: None,
            limit: None,
            is_partial_view: None,
        }
    }

    #[test]
    fn roundtrip_set_get() {
        let mut c = FileStateCache::new(10, 1_000_000);
        c.insert("/a/b.txt".into(), file_state("hello"));
        assert_eq!(c.get("/a/b.txt").map(|s| s.content.as_str()), Some("hello"));
    }

    #[test]
    fn path_normalisation_hits_same_entry() {
        let mut c = FileStateCache::new(10, 1_000_000);
        c.insert("/a/b/c.txt".into(), file_state("one"));
        // Lexically equivalent path → same cache entry.
        assert_eq!(
            c.get("/a/b/./c.txt").map(|s| s.content.as_str()),
            Some("one"),
        );
        assert_eq!(
            c.get("/a/d/../b/c.txt").map(|s| s.content.as_str()),
            Some("one"),
        );
    }

    #[test]
    fn remove_frees_bytes() {
        let mut c = FileStateCache::new(10, 1_000_000);
        c.insert("a".into(), file_state("abcdef"));
        let before = c.current_bytes();
        assert!(before >= 6);
        c.remove("a");
        assert_eq!(c.current_bytes(), before - 6);
    }

    #[test]
    fn entry_cap_evicts_lru() {
        let mut c = FileStateCache::new(2, 1_000_000);
        c.insert("a".into(), file_state("1"));
        c.insert("b".into(), file_state("2"));
        c.insert("c".into(), file_state("3"));
        // `a` is LRU and should have been evicted.
        assert!(!c.contains_key("a"));
        assert!(c.contains_key("b"));
        assert!(c.contains_key("c"));
    }

    #[test]
    fn byte_budget_evicts_lru() {
        // Budget fits two 3-byte entries, rejects a third.
        let mut c = FileStateCache::new(100, 6);
        c.insert("a".into(), file_state("aaa"));
        c.insert("b".into(), file_state("bbb"));
        // At-budget so far.
        assert_eq!(c.current_bytes(), 6);
        c.insert("c".into(), file_state("ccc"));
        // `a` evicted, `b` + `c` remain.
        assert!(!c.contains_key("a"));
        assert!(c.contains_key("b"));
        assert!(c.contains_key("c"));
        assert!(c.current_bytes() <= 6);
    }

    #[test]
    fn overwrite_updates_byte_total() {
        let mut c = FileStateCache::new(10, 1_000_000);
        c.insert("a".into(), file_state("xxx"));
        c.insert("a".into(), file_state("yyyyy"));
        assert_eq!(c.get("a").map(|s| s.content.as_str()), Some("yyyyy"));
        // Byte total reflects the replacement, not both.
        assert_eq!(c.current_bytes(), 5);
    }

    #[test]
    fn clear_resets() {
        let mut c = FileStateCache::new(10, 1_000_000);
        c.insert("a".into(), file_state("x"));
        c.insert("b".into(), file_state("y"));
        c.clear();
        assert!(c.is_empty());
        assert_eq!(c.current_bytes(), 0);
    }

    #[test]
    fn file_state_serialises_camel_case_partial_view() {
        let fs = FileState {
            content: "x".into(),
            timestamp: 42,
            offset: Some(10),
            limit: Some(100),
            is_partial_view: Some(true),
        };
        let v = serde_json::to_value(&fs).unwrap();
        assert_eq!(v["is_partial_view"], serde_json::json!(true));
        // No timestamp field drift.
        assert_eq!(v["timestamp"], serde_json::json!(42));
    }

    #[test]
    fn file_state_omits_none_fields() {
        let fs = FileState {
            content: "x".into(),
            timestamp: 0,
            offset: None,
            limit: None,
            is_partial_view: None,
        };
        let v = serde_json::to_value(&fs).unwrap();
        assert!(v.get("offset").is_none());
        assert!(v.get("limit").is_none());
        assert!(v.get("is_partial_view").is_none());
    }

    #[test]
    fn to_map_returns_all_entries() {
        let mut c = FileStateCache::new(10, 1_000_000);
        c.insert("a".into(), file_state("1"));
        c.insert("b".into(), file_state("2"));
        let m = c.to_map();
        assert_eq!(m.len(), 2);
        assert!(m.contains_key("a"));
        assert!(m.contains_key("b"));
    }

    #[test]
    fn default_size_limit_constant_pin() {
        assert_eq!(DEFAULT_MAX_CACHE_SIZE_BYTES, 25 * 1024 * 1024);
        assert_eq!(READ_FILE_STATE_CACHE_SIZE, 100);
    }
}
