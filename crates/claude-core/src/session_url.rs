//! Parse a session-resume identifier.
//!
//! Port of TS `src/utils/sessionUrl.ts`. `claude --resume` accepts
//! three identifier shapes:
//! 1. An absolute URL — e.g. `https://api.example.com/v1/session_ingress/session/UUID`
//!    → reconnect against the ingress URL with a fresh session ID.
//! 2. A plain UUID → resume that session in-place.
//! 3. A path ending in `.jsonl` → replay the transcript from disk
//!    (Windows absolute paths like `C:\...` parse as valid URLs, so
//!    the `.jsonl` check must run first).
//!
//! The Rust port uses `uuid::Uuid` and `url::Url` rather than the
//! TS runtime's `randomUUID` + WHATWG URL so the interpretation
//! matches downstream code.

use url::Url;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSessionUrl {
    pub session_id: Uuid,
    pub ingress_url: Option<String>,
    pub is_url: bool,
    pub jsonl_file: Option<String>,
    pub is_jsonl_file: bool,
}

/// Parse `resume_identifier`. Returns `None` when the string isn't
/// a `.jsonl` path, a valid UUID, or a parseable absolute URL.
pub fn parse_session_identifier(resume_identifier: &str) -> Option<ParsedSessionUrl> {
    // Check .jsonl BEFORE URL parsing: Windows absolute paths
    // (C:\path\file.jsonl) parse as valid URLs with C: as scheme.
    if resume_identifier.to_ascii_lowercase().ends_with(".jsonl") {
        return Some(ParsedSessionUrl {
            session_id: Uuid::new_v4(),
            ingress_url: None,
            is_url: false,
            jsonl_file: Some(resume_identifier.to_string()),
            is_jsonl_file: true,
        });
    }

    if let Ok(u) = Uuid::parse_str(resume_identifier) {
        return Some(ParsedSessionUrl {
            session_id: u,
            ingress_url: None,
            is_url: false,
            jsonl_file: None,
            is_jsonl_file: false,
        });
    }

    if let Ok(url) = Url::parse(resume_identifier) {
        return Some(ParsedSessionUrl {
            session_id: Uuid::new_v4(),
            ingress_url: Some(url.as_str().to_string()),
            is_url: true,
            jsonl_file: None,
            is_jsonl_file: false,
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_uuid_resumes_in_place() {
        let p = parse_session_identifier(
            "550e8400-e29b-41d4-a716-446655440000",
        )
        .unwrap();
        assert_eq!(
            p.session_id,
            Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap()
        );
        assert!(!p.is_url);
        assert!(!p.is_jsonl_file);
        assert!(p.ingress_url.is_none());
        assert!(p.jsonl_file.is_none());
    }

    #[test]
    fn url_routes_to_ingress_with_fresh_session() {
        let p = parse_session_identifier(
            "https://api.example.com/v1/session/ingress",
        )
        .unwrap();
        assert!(p.is_url);
        assert!(!p.is_jsonl_file);
        assert_eq!(
            p.ingress_url.as_deref(),
            Some("https://api.example.com/v1/session/ingress")
        );
    }

    #[test]
    fn jsonl_file_takes_precedence_over_url_parse() {
        // Windows absolute path parses as URL with `c:` scheme.
        // The .jsonl check must fire first.
        let p = parse_session_identifier(r"C:\Users\alex\sess.jsonl").unwrap();
        assert!(p.is_jsonl_file);
        assert_eq!(
            p.jsonl_file.as_deref(),
            Some(r"C:\Users\alex\sess.jsonl")
        );
        assert!(!p.is_url);
    }

    #[test]
    fn jsonl_suffix_is_case_insensitive() {
        let p = parse_session_identifier("/tmp/SESSION.JSONL").unwrap();
        assert!(p.is_jsonl_file);
    }

    #[test]
    fn garbage_returns_none() {
        assert!(parse_session_identifier("not a valid anything").is_none());
        assert!(parse_session_identifier("").is_none());
    }
}
