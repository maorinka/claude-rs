//! CCR v2 session-id tag compatibility helpers.
//!
//! Port of TS `src/bridge/sessionIdCompat.ts`. The compat API and infra API
//! can refer to the same underlying session with different prefixes:
//! `session_*` for v1/client-facing endpoints and `cse_*` for worker/infra
//! endpoints. The GrowthBook kill switch is modeled as an explicit boolean so
//! callers do not need global state.

/// Re-tag a `cse_*` session ID to `session_*` for compat/session endpoints.
pub fn to_compat_session_id(id: &str, cse_shim_enabled: bool) -> String {
    if !id.starts_with("cse_") || !cse_shim_enabled {
        return id.to_string();
    }
    format!("session_{}", &id["cse_".len()..])
}

/// Re-tag a `session_*` session ID to `cse_*` for infrastructure endpoints.
pub fn to_infra_session_id(id: &str) -> String {
    if !id.starts_with("session_") {
        return id.to_string();
    }
    format!("cse_{}", &id["session_".len()..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compat_session_id_follows_ts_shim_gate() {
        assert_eq!(to_compat_session_id("cse_abc123", true), "session_abc123");
        assert_eq!(to_compat_session_id("cse_abc123", false), "cse_abc123");
        assert_eq!(
            to_compat_session_id("session_abc123", true),
            "session_abc123"
        );
    }

    #[test]
    fn infra_session_id_retags_session_prefix_only() {
        assert_eq!(to_infra_session_id("session_abc123"), "cse_abc123");
        assert_eq!(to_infra_session_id("cse_abc123"), "cse_abc123");
        assert_eq!(to_infra_session_id("plain"), "plain");
    }
}
