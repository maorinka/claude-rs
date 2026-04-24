//! Tagged-ID encoder matching the API's `tagged_id.py` format.
//!
//! Port of TS `src/utils/taggedId.ts`. Produces IDs like
//! `user_01PaGUP2rbg1XDh7Z9W1CEpd` where the numeric body is a
//! fixed-length (22-char) base58 encoding of the 128-bit UUID
//! interpreted as an unsigned integer. Must stay in sync with
//! `api/api/common/utils/tagged_id.py`.
//!
//! Rust has native `u128`, so the TS BigInt path collapses to
//! straight integer arithmetic.

const BASE_58_CHARS: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
const VERSION: &str = "01";
/// `ceil(128 / log2(58)) = 22`.
const ENCODED_LENGTH: usize = 22;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaggedIdError {
    /// The UUID hex (after stripping dashes) was not exactly 32
    /// characters.
    InvalidUuidLength(usize),
    /// Hex digit out of range.
    InvalidHex,
}

impl std::fmt::Display for TaggedIdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidUuidLength(n) => {
                write!(f, "invalid UUID hex length: {n}")
            }
            Self::InvalidHex => f.write_str("invalid UUID hex digit"),
        }
    }
}

impl std::error::Error for TaggedIdError {}

/// Encode `value` as a fixed-width 22-char base58 string.
pub fn base58_encode_u128(value: u128) -> String {
    let base = BASE_58_CHARS.len() as u128;
    let mut buf = vec![BASE_58_CHARS[0]; ENCODED_LENGTH];
    let mut v = value;
    let mut i = ENCODED_LENGTH;
    while v > 0 && i > 0 {
        i -= 1;
        let rem = (v % base) as usize;
        buf[i] = BASE_58_CHARS[rem];
        v /= base;
    }
    String::from_utf8(buf).expect("base58 chars are ASCII")
}

/// Parse a UUID string (with or without dashes) into a `u128`.
pub fn uuid_to_u128(uuid: &str) -> Result<u128, TaggedIdError> {
    let hex: String = uuid.chars().filter(|c| *c != '-').collect();
    if hex.len() != 32 {
        return Err(TaggedIdError::InvalidUuidLength(hex.len()));
    }
    u128::from_str_radix(&hex, 16).map_err(|_| TaggedIdError::InvalidHex)
}

/// Encode an account UUID as a tagged ID (`{tag}_{version}{base58}`).
pub fn to_tagged_id(tag: &str, uuid: &str) -> Result<String, TaggedIdError> {
    let n = uuid_to_u128(uuid)?;
    Ok(format!("{tag}_{VERSION}{}", base58_encode_u128(n)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_encodes_to_leading_ones() {
        // Numeric 0 → all base58[0] chars → all '1'.
        let out = base58_encode_u128(0);
        assert_eq!(out.len(), ENCODED_LENGTH);
        assert_eq!(out, "1".repeat(ENCODED_LENGTH));
    }

    #[test]
    fn small_values_left_pad() {
        // Numeric 1 → one '2' at the tail, rest are '1'.
        let out = base58_encode_u128(1);
        assert_eq!(out.len(), ENCODED_LENGTH);
        assert_eq!(&out[..ENCODED_LENGTH - 1], &"1".repeat(ENCODED_LENGTH - 1));
        assert_eq!(&out[ENCODED_LENGTH - 1..], "2");
    }

    #[test]
    fn max_u128_encodes_to_fixed_width() {
        let out = base58_encode_u128(u128::MAX);
        assert_eq!(out.len(), ENCODED_LENGTH);
    }

    #[test]
    fn uuid_parses_with_and_without_dashes() {
        let with = uuid_to_u128("00000000-0000-0000-0000-000000000001").unwrap();
        let without = uuid_to_u128("00000000000000000000000000000001").unwrap();
        assert_eq!(with, 1);
        assert_eq!(without, 1);
    }

    #[test]
    fn uuid_rejects_bad_length() {
        let err = uuid_to_u128("abc").unwrap_err();
        assert_eq!(err, TaggedIdError::InvalidUuidLength(3));
    }

    #[test]
    fn uuid_rejects_bad_hex() {
        let err = uuid_to_u128("zzzzzzzz-zzzz-zzzz-zzzz-zzzzzzzzzzzz").unwrap_err();
        assert_eq!(err, TaggedIdError::InvalidHex);
    }

    #[test]
    fn tagged_id_shape() {
        let out = to_tagged_id("user", "00000000-0000-0000-0000-000000000000").unwrap();
        assert!(out.starts_with("user_01"));
        // tag + "_01" + 22 base58 chars = 7 + 22 = 29 for tag "user"
        assert_eq!(out.len(), "user_01".len() + ENCODED_LENGTH);
    }

    #[test]
    fn tagged_id_version_prefix_stable() {
        let out = to_tagged_id("org", "12345678-1234-1234-1234-123456789abc").unwrap();
        assert!(out.starts_with("org_01"));
    }

    #[test]
    fn encoding_roundtrips_via_decode() {
        // Sanity: encode, then decode via the published base58 alphabet.
        let n: u128 = 0x1234_5678_9ABC_DEF0_1122_3344_5566_7788;
        let enc = base58_encode_u128(n);
        let mut decoded: u128 = 0;
        let base = 58u128;
        for byte in enc.bytes() {
            let idx = BASE_58_CHARS
                .iter()
                .position(|&c| c == byte)
                .expect("char in alphabet");
            decoded = decoded * base + idx as u128;
        }
        assert_eq!(decoded, n);
    }
}
