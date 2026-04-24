//! Port of `src/constants/product.ts`.
//!
//! Product URLs + remote-session base URL resolution based on staging/local
//! markers baked into the session id or ingress URL.

pub const PRODUCT_URL: &str = "https://claude.com/claude-code";

pub const CLAUDE_AI_BASE_URL: &str = "https://claude.ai";
pub const CLAUDE_AI_STAGING_BASE_URL: &str = "https://claude-ai.staging.ant.dev";
pub const CLAUDE_AI_LOCAL_BASE_URL: &str = "http://localhost:4000";

/// True if the session looks like staging.
pub fn is_remote_session_staging(session_id: Option<&str>, ingress_url: Option<&str>) -> bool {
    session_id.is_some_and(|s| s.contains("_staging_"))
        || ingress_url.is_some_and(|u| u.contains("staging"))
}

/// True if the session looks like local dev.
pub fn is_remote_session_local(session_id: Option<&str>, ingress_url: Option<&str>) -> bool {
    session_id.is_some_and(|s| s.contains("_local_"))
        || ingress_url.is_some_and(|u| u.contains("localhost"))
}

/// Return the base URL for Claude.ai based on session environment.
pub fn get_claude_ai_base_url(session_id: Option<&str>, ingress_url: Option<&str>) -> &'static str {
    if is_remote_session_local(session_id, ingress_url) {
        CLAUDE_AI_LOCAL_BASE_URL
    } else if is_remote_session_staging(session_id, ingress_url) {
        CLAUDE_AI_STAGING_BASE_URL
    } else {
        CLAUDE_AI_BASE_URL
    }
}

/// Return the full session URL for a remote session: `<base>/code/<sid>`.
///
/// The TS port applies a `toCompatSessionId(sid)` shim that flips `cse_*`
/// prefixes to `session_*` for the claude.ai frontend — that bridge module
/// hasn't been ported yet. Until it is, callers pass the id they want
/// used; if a cse_ id leaks through the frontend will 400 and we'll
/// surface the bridge gap.
pub fn get_remote_session_url(session_id: &str, ingress_url: Option<&str>) -> String {
    let base = get_claude_ai_base_url(Some(session_id), ingress_url);
    format!("{}/code/{}", base, session_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staging_detected_from_session_id() {
        assert!(is_remote_session_staging(Some("session_staging_abc"), None));
        assert!(!is_remote_session_staging(Some("session_abc"), None));
    }

    #[test]
    fn local_wins_over_staging() {
        // Both markers present → local takes precedence in the ladder.
        let base = get_claude_ai_base_url(Some("session_local_staging_abc"), None);
        assert_eq!(base, CLAUDE_AI_LOCAL_BASE_URL);
    }

    #[test]
    fn prod_is_default() {
        assert_eq!(get_claude_ai_base_url(None, None), CLAUDE_AI_BASE_URL);
    }

    #[test]
    fn remote_session_url_shape() {
        let url = get_remote_session_url("session_abc", None);
        assert_eq!(url, "https://claude.ai/code/session_abc");
    }
}
