//! Work-secret decode + session URL helpers.
//!
//! Port of TS `src/bridge/workSecret.ts` — the pure-logic half.
//! `registerWorker` (HTTP POST to the CCR v2 worker-register endpoint)
//! lives with the bridge's HTTP client; it'll land with the rest of
//! the bridge runtime port.
//!
//! The TS bridge hands this module a base64url blob out of the CLI's
//! `work_secret` env var / flag; `decode_work_secret` parses it into
//! a typed `WorkSecret` so the bridge can reach session-ingress.
//! `build_sdk_url` / `build_ccr_v2_sdk_url` construct the two URL
//! shapes the bridge dials (WebSocket to session-ingress, HTTP to
//! CCR v2). `same_session_id` is the tagged-ID compat check that
//! lets a session accept its own work when the gateway returns a
//! `cse_*` tag but the client stored `session_*`.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkSecret {
    pub version: i64,
    pub session_ingress_token: String,
    pub api_base_url: String,
    /// Any other fields present in the work-secret blob are kept so
    /// re-serialisation round-trips losslessly.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkSecretError {
    /// base64url payload did not decode.
    InvalidBase64,
    /// decoded payload was not valid UTF-8 JSON.
    InvalidJson,
    /// `version` is missing, not 1, or unreadable.
    UnsupportedVersion(String),
    /// `session_ingress_token` missing or empty.
    MissingIngressToken,
    /// `api_base_url` missing.
    MissingApiBaseUrl,
}

impl fmt::Display for WorkSecretError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorkSecretError::InvalidBase64 => {
                f.write_str("work secret is not valid base64url")
            }
            WorkSecretError::InvalidJson => {
                f.write_str("work secret did not decode to JSON")
            }
            WorkSecretError::UnsupportedVersion(got) => {
                write!(f, "unsupported work secret version: {got}")
            }
            WorkSecretError::MissingIngressToken => f.write_str(
                "invalid work secret: missing or empty session_ingress_token",
            ),
            WorkSecretError::MissingApiBaseUrl => {
                f.write_str("invalid work secret: missing api_base_url")
            }
        }
    }
}

impl std::error::Error for WorkSecretError {}

/// Decode a base64url-encoded work secret. Matches TS
/// `decodeWorkSecret`: rejects anything that isn't `version === 1`
/// with a non-empty `session_ingress_token` and an `api_base_url`.
pub fn decode_work_secret(secret: &str) -> Result<WorkSecret, WorkSecretError> {
    let trimmed = secret.trim_end_matches('=');
    let bytes = URL_SAFE_NO_PAD
        .decode(trimmed)
        .map_err(|_| WorkSecretError::InvalidBase64)?;
    let text =
        std::str::from_utf8(&bytes).map_err(|_| WorkSecretError::InvalidJson)?;
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|_| WorkSecretError::InvalidJson)?;

    let obj = value
        .as_object()
        .ok_or_else(|| WorkSecretError::UnsupportedVersion("unknown".into()))?;

    match obj.get("version") {
        Some(v) if v == &serde_json::Value::from(1) => {}
        Some(other) => {
            return Err(WorkSecretError::UnsupportedVersion(other.to_string()));
        }
        None => {
            return Err(WorkSecretError::UnsupportedVersion("unknown".into()));
        }
    }

    match obj.get("session_ingress_token") {
        Some(serde_json::Value::String(s)) if !s.is_empty() => {}
        _ => return Err(WorkSecretError::MissingIngressToken),
    }

    if !matches!(obj.get("api_base_url"), Some(serde_json::Value::String(_))) {
        return Err(WorkSecretError::MissingApiBaseUrl);
    }

    serde_json::from_value(value).map_err(|_| WorkSecretError::InvalidJson)
}

/// Build a WebSocket SDK URL from an API base URL + session ID.
/// `ws://` + `/v2/` for localhost / 127.0.0.1 (direct to
/// session-ingress, no Envoy rewrite); `wss://` + `/v1/` elsewhere
/// (Envoy rewrites `/v1/` → `/v2/`).
pub fn build_sdk_url(api_base_url: &str, session_id: &str) -> String {
    let is_localhost =
        api_base_url.contains("localhost") || api_base_url.contains("127.0.0.1");
    let protocol = if is_localhost { "ws" } else { "wss" };
    let version = if is_localhost { "v2" } else { "v1" };
    let host = strip_scheme_and_trailing_slashes(api_base_url);
    format!("{protocol}://{host}/{version}/session_ingress/ws/{session_id}")
}

/// Build a CCR v2 session URL from an API base URL + session ID.
/// Unlike `build_sdk_url`, returns an HTTP(S) URL pointing at
/// `/v1/code/sessions/{id}`.
pub fn build_ccr_v2_sdk_url(api_base_url: &str, session_id: &str) -> String {
    let base = api_base_url.trim_end_matches('/');
    format!("{base}/v1/code/sessions/{session_id}")
}

fn strip_scheme_and_trailing_slashes(url: &str) -> String {
    let no_scheme = if let Some(rest) = url.strip_prefix("https://") {
        rest
    } else if let Some(rest) = url.strip_prefix("http://") {
        rest
    } else {
        url
    };
    no_scheme.trim_end_matches('/').to_string()
}

/// Tagged-ID compat: compare two session IDs ignoring a leading tag
/// prefix (`session_`, `cse_`, `session_staging_`, etc.). CCR v2's
/// compat layer returns `session_*` to v1 API clients but the work
/// poll response uses `cse_*`; both encode the same UUID.
///
/// Requires the body (suffix after the last `_`) to be ≥4 chars so
/// malformed IDs with tiny tag fragments don't collide.
pub fn same_session_id(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let a_body = a.rsplit_once('_').map(|(_, body)| body).unwrap_or(a);
    let b_body = b.rsplit_once('_').map(|(_, body)| body).unwrap_or(b);
    a_body.len() >= 4 && a_body == b_body
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn encode_secret(v: &serde_json::Value) -> String {
        URL_SAFE_NO_PAD.encode(v.to_string().as_bytes())
    }

    #[test]
    fn decode_valid_work_secret() {
        let blob = encode_secret(&json!({
            "version": 1,
            "session_ingress_token": "sk-ant-si-tok",
            "api_base_url": "https://api.example.com",
        }));
        let ws = decode_work_secret(&blob).expect("decodes");
        assert_eq!(ws.version, 1);
        assert_eq!(ws.session_ingress_token, "sk-ant-si-tok");
        assert_eq!(ws.api_base_url, "https://api.example.com");
    }

    #[test]
    fn preserves_extra_fields() {
        let blob = encode_secret(&json!({
            "version": 1,
            "session_ingress_token": "t",
            "api_base_url": "https://x",
            "custom": "value",
        }));
        let ws = decode_work_secret(&blob).expect("decodes");
        assert_eq!(
            ws.extra.get("custom"),
            Some(&serde_json::Value::from("value"))
        );
    }

    #[test]
    fn rejects_bad_base64() {
        let err = decode_work_secret("!!!").unwrap_err();
        assert_eq!(err, WorkSecretError::InvalidBase64);
    }

    #[test]
    fn rejects_non_json() {
        let blob = URL_SAFE_NO_PAD.encode(b"not json");
        let err = decode_work_secret(&blob).unwrap_err();
        assert_eq!(err, WorkSecretError::InvalidJson);
    }

    #[test]
    fn rejects_wrong_version() {
        let blob = encode_secret(&json!({
            "version": 2,
            "session_ingress_token": "t",
            "api_base_url": "https://x",
        }));
        let err = decode_work_secret(&blob).unwrap_err();
        match err {
            WorkSecretError::UnsupportedVersion(_) => {}
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn rejects_missing_token() {
        let blob = encode_secret(&json!({
            "version": 1,
            "session_ingress_token": "",
            "api_base_url": "https://x",
        }));
        let err = decode_work_secret(&blob).unwrap_err();
        assert_eq!(err, WorkSecretError::MissingIngressToken);
    }

    #[test]
    fn rejects_missing_api_base_url() {
        let blob = encode_secret(&json!({
            "version": 1,
            "session_ingress_token": "t",
        }));
        let err = decode_work_secret(&blob).unwrap_err();
        assert_eq!(err, WorkSecretError::MissingApiBaseUrl);
    }

    #[test]
    fn build_sdk_url_production_uses_wss_and_v1() {
        let url = build_sdk_url("https://api.anthropic.com", "sess-1");
        assert_eq!(
            url,
            "wss://api.anthropic.com/v1/session_ingress/ws/sess-1"
        );
    }

    #[test]
    fn build_sdk_url_localhost_uses_ws_and_v2() {
        let url = build_sdk_url("http://localhost:8080", "s");
        assert_eq!(url, "ws://localhost:8080/v2/session_ingress/ws/s");
        let url2 = build_sdk_url("http://127.0.0.1:9000", "s");
        assert_eq!(url2, "ws://127.0.0.1:9000/v2/session_ingress/ws/s");
    }

    #[test]
    fn build_sdk_url_strips_trailing_slashes() {
        let url = build_sdk_url("https://api.example.com///", "s");
        assert_eq!(url, "wss://api.example.com/v1/session_ingress/ws/s");
    }

    #[test]
    fn build_ccr_v2_sdk_url_basic() {
        let url = build_ccr_v2_sdk_url("https://api.example.com", "abc");
        assert_eq!(url, "https://api.example.com/v1/code/sessions/abc");
    }

    #[test]
    fn build_ccr_v2_strips_trailing_slashes() {
        let url = build_ccr_v2_sdk_url("https://api.example.com/", "abc");
        assert_eq!(url, "https://api.example.com/v1/code/sessions/abc");
    }

    #[test]
    fn same_session_id_identical() {
        assert!(same_session_id("session_abcd1234", "session_abcd1234"));
    }

    #[test]
    fn same_session_id_cross_tag() {
        assert!(same_session_id(
            "session_abcd1234efgh",
            "cse_abcd1234efgh"
        ));
    }

    #[test]
    fn same_session_id_staging_prefix() {
        // `session_staging_body` → rsplit on `_` gives `body`.
        assert!(same_session_id(
            "session_staging_abcd1234",
            "cse_abcd1234"
        ));
    }

    #[test]
    fn same_session_id_differs() {
        assert!(!same_session_id(
            "session_aaaaaaaa",
            "session_bbbbbbbb"
        ));
    }

    #[test]
    fn same_session_id_short_body_rejected() {
        // body "ab" is under the 4-char floor — don't collide.
        assert!(!same_session_id("session_ab", "cse_ab"));
    }

    #[test]
    fn same_session_id_bare_uuid_no_underscore() {
        // No underscore → whole string is the body.
        assert!(same_session_id("abcd1234", "abcd1234"));
        assert!(!same_session_id("abcd1234", "wxyz5678"));
    }
}
