//! UUID validation + agent-id generator helpers.
//!
//! Port of TS `utils/uuid.ts:1-27`.
//!
//! Kept separate from the existing `crate::agent_id` module because
//! that one handles team-scoped `agentName@teamName` formatting, while
//! TS `utils/uuid.ts` is the anonymous-agent case — random 8-byte
//! suffix with an optional label prefix.

use once_cell::sync::Lazy;
use rand::RngCore;
use regex::Regex;

/// Matches the canonical 8-4-4-4-12 hex-digit UUID shape, case
/// insensitive. TS `uuid.ts:4-5` pins the same pattern.
static UUID_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^(?i)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$").unwrap()
});

/// Returns `Some(s)` if `maybe_uuid` is a syntactically valid UUID,
/// `None` otherwise. Does NOT check version bits or variant — just the
/// 8-4-4-4-12 hex shape, matching TS which uses `uuid.ts:5`'s regex.
pub fn validate_uuid(maybe_uuid: &str) -> Option<&str> {
    if UUID_PATTERN.is_match(maybe_uuid) {
        Some(maybe_uuid)
    } else {
        None
    }
}

/// Generate a new agent ID with an `a` prefix + optional label.
/// Format: `a<label>-<16 hex chars>` when `label` is given, else
/// `a<16 hex chars>`. The 16 hex chars encode 8 random bytes.
///
/// TS uses Node's `crypto.randomBytes(8)`; Rust uses
/// `rand::rngs::OsRng` via `rand::RngCore::fill_bytes`, which is the
/// same OS CSPRNG source.
pub fn create_agent_id(label: Option<&str>) -> String {
    let mut bytes = [0u8; 8];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    match label {
        Some(l) => format!("a{l}-{hex}"),
        None => format!("a{hex}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_canonical_uuid_v4_shape() {
        assert!(validate_uuid("550e8400-e29b-41d4-a716-446655440000").is_some());
    }

    #[test]
    fn accepts_uppercase() {
        // TS regex has the `i` flag — upper-case hex must also pass.
        assert!(validate_uuid("550E8400-E29B-41D4-A716-446655440000").is_some());
    }

    #[test]
    fn accepts_mixed_case() {
        assert!(validate_uuid("550e8400-E29b-41D4-a716-446655440000").is_some());
    }

    #[test]
    fn rejects_wrong_segment_lengths() {
        assert!(validate_uuid("550e8400-e29b-41d4-a716-44665544000").is_none());
        assert!(validate_uuid("550e8400-e29b-41d4-a716-4466554400001").is_none());
        assert!(validate_uuid("550e840-e29b-41d4-a716-446655440000").is_none());
    }

    #[test]
    fn rejects_non_hex_chars() {
        assert!(validate_uuid("550e8400-e29b-41d4-a716-44665544000Z").is_none());
        assert!(validate_uuid("------------------------").is_none());
    }

    #[test]
    fn rejects_empty_and_arbitrary_strings() {
        assert!(validate_uuid("").is_none());
        assert!(validate_uuid("hello world").is_none());
        assert!(validate_uuid("not-a-uuid").is_none());
    }

    #[test]
    fn rejects_leading_or_trailing_whitespace() {
        // TS regex is anchored with `^…$`, so trimming is the caller's job.
        assert!(validate_uuid(" 550e8400-e29b-41d4-a716-446655440000").is_none());
        assert!(validate_uuid("550e8400-e29b-41d4-a716-446655440000 ").is_none());
    }

    #[test]
    fn create_agent_id_without_label() {
        let id = create_agent_id(None);
        // Format: `a` + 16 hex chars = 17 total.
        assert_eq!(id.len(), 17);
        assert!(id.starts_with('a'));
        assert!(id[1..].chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn create_agent_id_with_label() {
        let id = create_agent_id(Some("compact"));
        // Format: `a<label>-<16 hex>` — `a` + "compact" + `-` + 16 = 25.
        assert_eq!(id.len(), "acompact-".len() + 16);
        assert!(id.starts_with("acompact-"));
        let hex_part = &id[9..];
        assert_eq!(hex_part.len(), 16);
        assert!(hex_part.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn create_agent_id_is_random() {
        // 2^64 collision space — a handful of consecutive calls should
        // never collide unless the RNG is broken.
        let ids: std::collections::HashSet<_> = (0..8).map(|_| create_agent_id(None)).collect();
        assert_eq!(ids.len(), 8);
    }

    #[test]
    fn create_agent_id_label_preserves_case() {
        // TS doesn't case-fold the label. Rust must preserve it too so
        // case-sensitive downstream matchers (e.g. log greps) work.
        let id = create_agent_id(Some("Dream"));
        assert!(id.starts_with("aDream-"));
    }
}
