//! Unauthenticated JWT payload + expiry decoding.
//!
//! Port of TS `src/bridge/jwtUtils.ts` — pure decoding helpers only.
//! The scheduler half of the TS file (`createTokenRefreshScheduler`)
//! owns timers tied to bridge session state, getAccessToken wiring,
//! and analytics logging; it lives with the bridge runtime layer
//! that hasn't landed on the Rust side yet. This patch ports the
//! decoder it depends on so when the scheduler port happens, both
//! sides read the `exp` claim the same way.
//!
//! Signature verification is intentionally NOT performed — these
//! helpers are used to decide *when* to refresh a token, not to
//! trust its contents. Call sites that need to authenticate the
//! bearer must do so through the bridge API layer.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use serde_json::Value;

/// TS `sk-ant-si-` prefix for session-ingress tokens. Stripped
/// before base64-decoding the JWT payload so tokens wrapped with
/// the Anthropic session-ingress prefix decode cleanly.
pub const SESSION_INGRESS_PREFIX: &str = "sk-ant-si-";

/// Decode a JWT payload segment **without verifying the signature**.
/// Strips the `sk-ant-si-` prefix if present. Returns `None` when:
/// - the token is not three dot-separated segments,
/// - the payload segment is empty,
/// - the base64url payload does not decode,
/// - the decoded bytes are not valid UTF-8, or
/// - the UTF-8 is not a JSON value.
pub fn decode_jwt_payload(token: &str) -> Option<Value> {
    let jwt = token.strip_prefix(SESSION_INGRESS_PREFIX).unwrap_or(token);
    let mut parts = jwt.split('.');
    let _header = parts.next()?;
    let payload = parts.next()?;
    let _signature = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    if payload.is_empty() {
        return None;
    }
    // base64url, possibly with trailing `=` padding — strip so the
    // NO_PAD engine accepts either form.
    let trimmed = payload.trim_end_matches('=');
    let bytes = URL_SAFE_NO_PAD.decode(trimmed).ok()?;
    let text = std::str::from_utf8(&bytes).ok()?;
    serde_json::from_str(text).ok()
}

/// Decode the `exp` claim without verifying the signature. Returns
/// Unix seconds as `i64` so callers doing arithmetic with `now`
/// timestamps don't need to cast. `None` if the claim is missing,
/// non-numeric, or outside i64 range.
pub fn decode_jwt_expiry(token: &str) -> Option<i64> {
    let payload = decode_jwt_payload(token)?;
    let exp = payload.get("exp")?;
    exp.as_i64().or_else(|| exp.as_f64().map(|f| f as i64))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_token(payload: &Value) -> String {
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"none","typ":"JWT"}"#);
        let body = URL_SAFE_NO_PAD.encode(payload.to_string().as_bytes());
        let signature = URL_SAFE_NO_PAD.encode(b"sig");
        format!("{header}.{body}.{signature}")
    }

    #[test]
    fn decodes_plain_jwt_payload() {
        let payload = json!({"sub": "user-1", "exp": 1_700_000_000});
        let token = make_token(&payload);
        let decoded = decode_jwt_payload(&token).expect("decodes");
        assert_eq!(decoded, payload);
    }

    #[test]
    fn strips_session_ingress_prefix() {
        let payload = json!({"exp": 42});
        let token = format!("{SESSION_INGRESS_PREFIX}{}", make_token(&payload));
        let decoded = decode_jwt_payload(&token).expect("decodes");
        assert_eq!(decoded["exp"], 42);
    }

    #[test]
    fn rejects_wrong_segment_count() {
        assert!(decode_jwt_payload("only.two").is_none());
        assert!(decode_jwt_payload("a.b.c.d").is_none());
    }

    #[test]
    fn rejects_empty_payload_segment() {
        assert!(decode_jwt_payload("a..c").is_none());
    }

    #[test]
    fn rejects_non_base64_payload() {
        assert!(decode_jwt_payload("a.!!!!.c").is_none());
    }

    #[test]
    fn rejects_non_json_payload() {
        let payload_b64 = URL_SAFE_NO_PAD.encode(b"not json");
        let token = format!("header.{payload_b64}.sig");
        assert!(decode_jwt_payload(&token).is_none());
    }

    #[test]
    fn extracts_expiry_as_i64() {
        let payload = json!({"exp": 1_700_000_000_i64});
        let token = make_token(&payload);
        assert_eq!(decode_jwt_expiry(&token), Some(1_700_000_000));
    }

    #[test]
    fn expiry_from_float_claim_truncated() {
        let payload = json!({"exp": 1_700_000_000.5});
        let token = make_token(&payload);
        assert_eq!(decode_jwt_expiry(&token), Some(1_700_000_000));
    }

    #[test]
    fn expiry_missing_returns_none() {
        let payload = json!({"sub": "no-exp"});
        let token = make_token(&payload);
        assert_eq!(decode_jwt_expiry(&token), None);
    }

    #[test]
    fn expiry_non_numeric_returns_none() {
        let payload = json!({"exp": "tomorrow"});
        let token = make_token(&payload);
        assert_eq!(decode_jwt_expiry(&token), None);
    }

    #[test]
    fn decode_with_padded_payload() {
        // Node's Buffer.from(x, 'base64url') accepts padded input too;
        // the TS wrapper relies on that, so we match.
        let payload = json!({"exp": 1});
        let header = URL_SAFE_NO_PAD.encode(b"{}");
        let body = URL_SAFE_NO_PAD.encode(payload.to_string().as_bytes());
        let token = format!("{header}.{body}==.sig");
        assert_eq!(decode_jwt_expiry(&token), Some(1));
    }
}
