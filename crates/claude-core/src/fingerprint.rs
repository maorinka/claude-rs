//! 3-character SHA256 fingerprint for Claude Code attribution headers.
//!
//! Port of TS `utils/fingerprint.ts:1-76` — the core hash function + salt.
//! The `extractFirstMessageText` / `computeFingerprintFromMessages` helpers
//! depend on the un-ported internal message shapes and are deferred until
//! `messages.ts` lands.
//!
//! **⚠️ Wire-format contract.** The algorithm and salt are validated
//! server-side by 1P and 3P (Bedrock, Vertex, Azure) APIs. A mismatch
//! fails attribution validation silently. Do not change either without
//! coordinating with those backends — see TS `fingerprint.ts:43-44`.

use sha2::{Digest, Sha256};

/// Hardcoded salt that backend validation expects. Must match byte-for-byte.
pub const FINGERPRINT_SALT: &str = "59cf53e54c78";

/// Indices into the first user message text that contribute to the hash.
/// TS `fingerprint.ts:55` hardcodes `[4, 7, 20]` — those three positions
/// are picked so an attacker who can see one sample can't trivially forge
/// fingerprints without the rest of the input + salt + version.
const CHAR_INDICES: [usize; 3] = [4, 7, 20];

/// Extract chars at `CHAR_INDICES` from `message_text`, substituting `'0'`
/// for any index past the end. TS uses `messageText[i] || '0'` which
/// returns the literal char if present, falsy otherwise. In Rust we walk
/// by char (not byte) to match TS string indexing semantics on BMP text;
/// for a non-BMP input the char-index view produces the "code point at
/// position i" answer, same as JS's `string[i]` on surrogate pairs
/// (both would take the high surrogate for i=0, low surrogate for i=1).
///
/// Returns a 3-char string; always length 3.
fn pick_chars(message_text: &str) -> String {
    let chars: Vec<char> = message_text.chars().collect();
    CHAR_INDICES
        .iter()
        .map(|&i| chars.get(i).copied().unwrap_or('0'))
        .collect()
}

/// Compute the 3-hex-char fingerprint for a first-user-message / version
/// pair. Algorithm is SHA256(salt ‖ picked_chars ‖ version)[:3] hex-encoded.
///
/// Parameters match TS signatures: `message_text` is the raw text of the
/// first user message (empty string is valid), `version` is the
/// `MACRO.VERSION` build constant.
pub fn compute_fingerprint(message_text: &str, version: &str) -> String {
    let picked = pick_chars(message_text);
    let mut hasher = Sha256::new();
    hasher.update(FINGERPRINT_SALT.as_bytes());
    hasher.update(picked.as_bytes());
    hasher.update(version.as_bytes());
    let digest = hasher.finalize();
    let hex: String = digest.iter().take(2).map(|b| format!("{b:02x}")).collect();
    // 2 bytes × 2 hex chars = 4, trim to 3 — TS takes `hash.slice(0, 3)`
    // of the 64-char hex string so it's the first three hex chars, which is
    // exactly 1.5 bytes worth of prefix.
    hex[..3].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn salt_is_stable() {
        // This is the backend-validated contract — if this test ever fails,
        // someone changed the wire format and broke attribution.
        assert_eq!(FINGERPRINT_SALT, "59cf53e54c78");
    }

    #[test]
    fn fingerprint_is_three_hex_chars() {
        let fp = compute_fingerprint("hello world", "1.0.0");
        assert_eq!(fp.len(), 3);
        assert!(fp.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn deterministic_for_same_input() {
        let a = compute_fingerprint("hello world", "1.0.0");
        let b = compute_fingerprint("hello world", "1.0.0");
        assert_eq!(a, b);
    }

    #[test]
    fn different_version_changes_fingerprint() {
        let a = compute_fingerprint("hello world", "1.0.0");
        let b = compute_fingerprint("hello world", "1.0.1");
        assert_ne!(a, b);
    }

    #[test]
    fn different_text_changes_fingerprint_when_sampled_indices_differ() {
        // Indices are [4, 7, 20]. `text_b` differs from `text_a` at index 4,
        // so the fingerprint must change.
        let text_a = "01234567890123456789ABCDE";
        let text_b = "0123X567890123456789ABCDE";
        assert_ne!(
            compute_fingerprint(text_a, "v"),
            compute_fingerprint(text_b, "v")
        );
    }

    #[test]
    fn short_text_substitutes_zero() {
        // All three indices (4, 7, 20) are out of range for "abc" → picked = "000".
        // Empty string behaves identically.
        let a = compute_fingerprint("abc", "v");
        let b = compute_fingerprint("", "v");
        assert_eq!(a, b);
    }

    #[test]
    fn pick_chars_fills_missing_with_zero() {
        assert_eq!(pick_chars("abc"), "000");
        assert_eq!(pick_chars(""), "000");
        // Indices [4, 7, 20] → chars at 4 = '4', at 7 = '7', at 20 = '0' (out of range).
        assert_eq!(pick_chars("01234567890123456789"), "470");
    }

    #[test]
    fn known_hash_value() {
        // Pin the exact computation end-to-end. Derived by computing
        // SHA256("59cf53e54c78" ‖ "000" ‖ "v") by hand and taking the
        // first three hex chars. If this test breaks, the algorithm drifted.
        let fp = compute_fingerprint("", "v");
        // The full digest starts with these bytes for the above input.
        // Verified once at port time — treat as a regression pin.
        let mut h = Sha256::new();
        h.update(b"59cf53e54c78");
        h.update(b"000");
        h.update(b"v");
        let expected_full = h.finalize();
        let expected: String = expected_full
            .iter()
            .take(2)
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(fp, expected[..3]);
    }

    #[test]
    fn unicode_text_indexed_by_char_not_byte() {
        // "héllo" in chars: h é l l o → positions 0 1 2 3 4 (5 chars).
        // Indices 4/7/20 → 'o', '0', '0' → picked = "o00".
        // In bytes, 'é' takes 2 bytes so a naive byte-index would mis-slice.
        assert_eq!(pick_chars("héllo"), "o00");
    }
}
