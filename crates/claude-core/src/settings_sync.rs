//! User-settings sync data types + diff helpers.
//!
//! Port of the shape + helper half of `src/services/settingsSync/`.
//! TS ships 648 LOC total: axios uploads/downloads, OAuth refresh,
//! retry backoff, per-file size caps, feature-gated guards. The bulk
//! of those depend on the Rust OAuth / HTTP stack gaining a sync
//! endpoint first — so this module ports the stable pieces the API
//! client will build on top of:
//!
//!   - `UserSyncContent` / `UserSyncData` shapes
//!   - `SYNC_KEYS` string constants
//!   - `content_diff` — compute which keys changed between two
//!     snapshots (upload path needs this to be incremental)
//!   - `content_checksum` — MD5 over a deterministic serialisation
//!     of the content map (matches TS `checksum` field)
//!
//! Callers wiring the HTTP layer use these types unchanged.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Per-file size cap enforced by the backend. Matches TS
/// `MAX_FILE_SIZE_BYTES = 500 * 1024`.
pub const MAX_FILE_SIZE_BYTES: usize = 500 * 1024;

/// Stable keys used to address sync entries. Match TS SYNC_KEYS.
pub const SYNC_KEY_USER_SETTINGS: &str = "~/.claude/settings.json";
pub const SYNC_KEY_USER_MEMORY: &str = "~/.claude/CLAUDE.md";

pub fn project_settings_key(project_id: &str) -> String {
    format!("projects/{}/.claude/settings.local.json", project_id)
}

pub fn project_memory_key(project_id: &str) -> String {
    format!("projects/{}/CLAUDE.local.md", project_id)
}

/// Flat key→content map. Keys are opaque strings (typically file
/// paths). Values are UTF-8 content (JSON / Markdown).
///
/// BTreeMap keeps iteration deterministic so `content_checksum`
/// produces a stable hash.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct UserSyncContent {
    pub entries: BTreeMap<String, String>,
}

/// Full shape returned by GET /api/claude_code/user_settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserSyncData {
    #[serde(rename = "userId")]
    pub user_id: String,
    pub version: u64,
    /// ISO 8601 timestamp.
    #[serde(rename = "lastModified")]
    pub last_modified: String,
    /// MD5 hex of the content map.
    pub checksum: String,
    pub content: UserSyncContent,
}

/// Diff between two content snapshots — returned as (key, new_value)
/// pairs for every key whose value changed or was added in `new`.
/// Keys absent from `new` are not reported (callers that need delete
/// tracking should use `removed_keys`).
pub fn content_diff<'a>(
    old: &UserSyncContent,
    new: &'a UserSyncContent,
) -> Vec<(&'a String, &'a String)> {
    let mut out = Vec::new();
    for (k, v) in &new.entries {
        match old.entries.get(k) {
            Some(prev) if prev == v => continue,
            _ => out.push((k, v)),
        }
    }
    out
}

/// Keys present in `old` but missing in `new`.
pub fn removed_keys<'a>(old: &'a UserSyncContent, new: &UserSyncContent) -> Vec<&'a String> {
    old.entries
        .keys()
        .filter(|k| !new.entries.contains_key(*k))
        .collect()
}

/// MD5-hex checksum over a deterministic serialisation of the
/// content map. Matches the TS backend's `checksum` expectation —
/// the map is iterated in sorted key order (BTreeMap) and each
/// entry is written as `<key>\n<value>\n\x00`.
pub fn content_checksum(content: &UserSyncContent) -> String {
    // Implement MD5 inline so we don't add a new crate dep. Public
    // domain reference implementation.
    md5_hex(&canonicalise(content))
}

fn canonicalise(content: &UserSyncContent) -> Vec<u8> {
    let mut buf = Vec::new();
    for (k, v) in &content.entries {
        buf.extend_from_slice(k.as_bytes());
        buf.push(b'\n');
        buf.extend_from_slice(v.as_bytes());
        buf.push(b'\n');
        buf.push(0);
    }
    buf
}

fn md5_hex(data: &[u8]) -> String {
    let d = md5_digest(data);
    let mut out = String::with_capacity(32);
    for b in d {
        out.push_str(&format!("{:02x}", b));
    }
    out
}

// ── Minimal MD5 (RFC 1321) — from the public-domain reference impl ─────────

fn md5_digest(data: &[u8]) -> [u8; 16] {
    let mut msg = data.to_vec();
    let original_len_bits = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&original_len_bits.to_le_bytes());

    let mut a0: u32 = 0x67452301;
    let mut b0: u32 = 0xefcdab89;
    let mut c0: u32 = 0x98badcfe;
    let mut d0: u32 = 0x10325476;

    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20,
        5, 9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23,
        6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    const K: [u32; 64] = [
        0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613,
        0xfd469501, 0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193,
        0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d,
        0x02441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed,
        0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122,
        0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa,
        0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244,
        0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
        0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb,
        0xeb86d391,
    ];

    for chunk in msg.chunks_exact(64) {
        let mut m = [0u32; 16];
        for i in 0..16 {
            m[i] = u32::from_le_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = match i {
                0..=15 => ((b & c) | ((!b) & d), i),
                16..=31 => ((d & b) | ((!d) & c), (5 * i + 1) % 16),
                32..=47 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let temp = d;
            d = c;
            c = b;
            b = b.wrapping_add(
                a.wrapping_add(f)
                    .wrapping_add(K[i])
                    .wrapping_add(m[g])
                    .rotate_left(S[i]),
            );
            a = temp;
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }

    let mut out = [0u8; 16];
    out[..4].copy_from_slice(&a0.to_le_bytes());
    out[4..8].copy_from_slice(&b0.to_le_bytes());
    out[8..12].copy_from_slice(&c0.to_le_bytes());
    out[12..].copy_from_slice(&d0.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_entries<'a>(pairs: &[(&'a str, &'a str)]) -> UserSyncContent {
        let mut m = BTreeMap::new();
        for (k, v) in pairs {
            m.insert((*k).to_string(), (*v).to_string());
        }
        UserSyncContent { entries: m }
    }

    #[test]
    fn sync_key_constants() {
        assert_eq!(SYNC_KEY_USER_SETTINGS, "~/.claude/settings.json");
        assert_eq!(SYNC_KEY_USER_MEMORY, "~/.claude/CLAUDE.md");
        assert_eq!(
            project_settings_key("abc"),
            "projects/abc/.claude/settings.local.json"
        );
        assert_eq!(project_memory_key("abc"), "projects/abc/CLAUDE.local.md");
    }

    #[test]
    fn diff_surfaces_changed_and_added() {
        let old = with_entries(&[("a", "1"), ("b", "2")]);
        let new = with_entries(&[("a", "1"), ("b", "3"), ("c", "4")]);
        let d = content_diff(&old, &new);
        let d_map: std::collections::BTreeMap<_, _> = d.into_iter().collect();
        assert_eq!(d_map.len(), 2);
        assert_eq!(d_map.get(&"b".to_string()), Some(&&"3".to_string()));
        assert_eq!(d_map.get(&"c".to_string()), Some(&&"4".to_string()));
    }

    #[test]
    fn removed_keys_reports_gone_keys() {
        let old = with_entries(&[("a", "1"), ("b", "2")]);
        let new = with_entries(&[("a", "1")]);
        let r = removed_keys(&old, &new);
        assert_eq!(r, vec![&"b".to_string()]);
    }

    #[test]
    fn checksum_stable_across_insertion_order() {
        let mut a = BTreeMap::new();
        a.insert("a".into(), "1".into());
        a.insert("b".into(), "2".into());
        let mut b = BTreeMap::new();
        b.insert("b".into(), "2".into());
        b.insert("a".into(), "1".into());
        let ca = content_checksum(&UserSyncContent { entries: a });
        let cb = content_checksum(&UserSyncContent { entries: b });
        assert_eq!(ca, cb);
    }

    #[test]
    fn checksum_changes_with_content() {
        let v1 = with_entries(&[("a", "1")]);
        let v2 = with_entries(&[("a", "2")]);
        assert_ne!(content_checksum(&v1), content_checksum(&v2));
    }

    #[test]
    fn md5_empty_string_matches_reference() {
        // Known MD5: d41d8cd98f00b204e9800998ecf8427e
        let out = md5_hex(&[]);
        assert_eq!(out, "d41d8cd98f00b204e9800998ecf8427e");
    }

    #[test]
    fn md5_abc_matches_reference() {
        let out = md5_hex(b"abc");
        assert_eq!(out, "900150983cd24fb0d6963f7d28e17f72");
    }

    #[test]
    fn empty_content_checksum_is_md5_of_empty() {
        let empty = UserSyncContent {
            entries: BTreeMap::new(),
        };
        assert_eq!(content_checksum(&empty), md5_hex(&[]));
    }
}
