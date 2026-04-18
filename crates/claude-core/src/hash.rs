//! String + content hashes.
//!
//! Port of TS `src/utils/hash.ts`. The TS file has a runtime-switch
//! path that prefers `Bun.hash` (wyhash) when running under Bun.
//! Rust has no Bun fallback, so `hash_content` / `hash_pair` always
//! use SHA-256 — which is what the TS Node path does anyway. The
//! `djb2` helper is kept for stable on-disk hashes (cache dir
//! names that must survive runtime upgrades), matching TS intent.

use sha2::{Digest, Sha256};

/// Deterministic djb2 string hash returning a signed 32-bit int
/// cast into i32. Matches the TS implementation bit-for-bit so
/// outputs round-trip between runtimes.
pub fn djb2_hash(s: &str) -> i32 {
    let mut hash: i32 = 0;
    for ch in s.chars() {
        // TS: `((hash << 5) - hash + charCodeAt(i)) | 0`.
        // `charCodeAt` returns the UTF-16 code unit, not the codepoint.
        // For non-BMP chars TS hashes each surrogate half; we
        // emulate that by iterating over UTF-16 code units.
        let mut buf = [0u16; 2];
        let units = ch.encode_utf16(&mut buf);
        for &unit in units.iter() {
            hash = hash
                .wrapping_shl(5)
                .wrapping_sub(hash)
                .wrapping_add(unit as i32);
        }
    }
    hash
}

/// Hash arbitrary content for change detection. SHA-256 hex. Not
/// crypto-safe for authentication; intended for diff detection.
pub fn hash_content(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hex_encode(&hasher.finalize())
}

/// Hash two strings without allocating an intermediate concat.
/// Uses a NUL byte as a separator so `("ab", "c")` and `("a", "bc")`
/// don't collide. Matches the TS Node path (incremental
/// SHA-256 update with a `\0` separator).
pub fn hash_pair(a: &str, b: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(a.as_bytes());
    hasher.update(b"\0");
    hasher.update(b.as_bytes());
    hex_encode(&hasher.finalize())
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(nibble(b >> 4));
        s.push(nibble(b & 0x0f));
    }
    s
}

fn nibble(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        10..=15 => (b'a' + (n - 10)) as char,
        _ => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn djb2_empty_string_is_zero() {
        assert_eq!(djb2_hash(""), 0);
    }

    #[test]
    fn djb2_matches_reference_values() {
        // Outputs captured by running the TS reference
        // implementation under Node — see the commit message for
        // the snippet. Changing any of these means the algorithm
        // diverged from TS.
        assert_eq!(djb2_hash("a"), 97);
        assert_eq!(djb2_hash("hello"), 99_162_322);
        assert_eq!(djb2_hash("Hello, World!"), 1_498_789_909);
    }

    #[test]
    fn djb2_wraps_on_overflow() {
        // Any long input eventually wraps; the result must stay a
        // valid i32 (never panic).
        let long = "x".repeat(10_000);
        let _ = djb2_hash(&long);
    }

    #[test]
    fn hash_content_is_sha256_hex() {
        // sha256("") = e3b0c4...
        assert_eq!(
            hash_content(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        // sha256("abc") = ba7816...
        assert_eq!(
            hash_content("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn hash_pair_disambiguates_splits() {
        // ("ab", "c") and ("a", "bc") must not collide thanks to
        // the \0 separator.
        let ab_c = hash_pair("ab", "c");
        let a_bc = hash_pair("a", "bc");
        assert_ne!(ab_c, a_bc);
    }

    #[test]
    fn hash_pair_is_deterministic() {
        let a = hash_pair("foo", "bar");
        let b = hash_pair("foo", "bar");
        assert_eq!(a, b);
    }
}
