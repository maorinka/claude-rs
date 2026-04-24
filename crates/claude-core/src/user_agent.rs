//! User-Agent string helpers.
//!
//! Port of `src/utils/userAgent.ts` + the string-building pieces of
//! `src/utils/http.ts`. The TS `MACRO.VERSION` is a build-time macro
//! substituted from package.json; the Rust side uses
//! `env!("CARGO_PKG_VERSION")` at compile time. Auth-header helpers
//! (getAuthHeaders, withOAuth401Retry) depend on the full auth pipeline
//! and stay in the existing claude_core::auth module — this module
//! ships only the UA strings.

/// Base user-agent string used by most clients. Matches TS
/// `getClaudeCodeUserAgent`.
pub fn get_claude_code_user_agent() -> String {
    format!("claude-code/{}", env!("CARGO_PKG_VERSION"))
}

/// Full user agent sent to the Anthropic API. The `claude-cli` prefix
/// is load-bearing — log filtering matches on it. Do NOT change
/// without updating the log pipeline.
///
/// Matches TS `getUserAgent`. Appends optional tags:
///   - `agent-sdk/<version>` when `CLAUDE_AGENT_SDK_VERSION` is set
///   - `client-app/<name>` when `CLAUDE_AGENT_SDK_CLIENT_APP` is set
///   - `workload/<w>` when the caller-provided `workload` is non-empty
///
/// Unlike TS, `workload` is passed explicitly instead of read from a
/// thread-local — Rust callers typically pass it through a context
/// struct and a clean signature avoids needing a workload singleton.
pub fn get_user_agent(workload: Option<&str>) -> String {
    let user_type = std::env::var("USER_TYPE").unwrap_or_default();
    let entrypoint = std::env::var("CLAUDE_CODE_ENTRYPOINT").unwrap_or_else(|_| "cli".into());
    let agent_sdk_version = std::env::var("CLAUDE_AGENT_SDK_VERSION")
        .map(|v| format!(", agent-sdk/{}", v))
        .unwrap_or_default();
    let client_app = std::env::var("CLAUDE_AGENT_SDK_CLIENT_APP")
        .map(|v| format!(", client-app/{}", v))
        .unwrap_or_default();
    let workload_suffix = match workload {
        Some(w) if !w.is_empty() => format!(", workload/{}", w),
        _ => String::new(),
    };

    format!(
        "claude-cli/{} ({}, {}{}{}{})",
        env!("CARGO_PKG_VERSION"),
        user_type,
        entrypoint,
        agent_sdk_version,
        client_app,
        workload_suffix,
    )
}

/// MCP client UA sent when Claude Code reaches MCP servers.
/// Matches TS `getMCPUserAgent` — parenthetical is assembled only when
/// at least one tag exists.
pub fn get_mcp_user_agent() -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Ok(e) = std::env::var("CLAUDE_CODE_ENTRYPOINT") {
        parts.push(e);
    }
    if let Ok(v) = std::env::var("CLAUDE_AGENT_SDK_VERSION") {
        parts.push(format!("agent-sdk/{}", v));
    }
    if let Ok(c) = std::env::var("CLAUDE_AGENT_SDK_CLIENT_APP") {
        parts.push(format!("client-app/{}", c));
    }
    let suffix = if parts.is_empty() {
        String::new()
    } else {
        format!(" ({})", parts.join(", "))
    };
    format!("claude-code/{}{}", env!("CARGO_PKG_VERSION"), suffix)
}

/// User-Agent for WebFetch requests to arbitrary sites.
/// `Claude-User` is Anthropic's publicly documented agent for
/// user-initiated fetches — site operators key off it in robots.txt.
/// The claude-code suffix distinguishes local CLI traffic from
/// claude.ai server-side fetches.
pub fn get_web_fetch_user_agent() -> String {
    format!(
        "Claude-User ({}; +https://support.anthropic.com/)",
        get_claude_code_user_agent()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn clear_env() {
        for k in &[
            "USER_TYPE",
            "CLAUDE_CODE_ENTRYPOINT",
            "CLAUDE_AGENT_SDK_VERSION",
            "CLAUDE_AGENT_SDK_CLIENT_APP",
        ] {
            std::env::remove_var(k);
        }
    }

    #[test]
    fn claude_code_ua_includes_package_version() {
        let ua = get_claude_code_user_agent();
        assert!(ua.starts_with("claude-code/"));
        assert!(ua.len() > "claude-code/".len());
    }

    #[test]
    fn web_fetch_ua_wraps_claude_user() {
        let ua = get_web_fetch_user_agent();
        assert!(ua.starts_with("Claude-User ("));
        assert!(ua.contains("claude-code/"));
        assert!(ua.contains("support.anthropic.com"));
    }

    #[test]
    fn user_agent_respects_env_tags() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        std::env::set_var("USER_TYPE", "external");
        std::env::set_var("CLAUDE_CODE_ENTRYPOINT", "sdk");
        std::env::set_var("CLAUDE_AGENT_SDK_VERSION", "1.0.0");
        std::env::set_var("CLAUDE_AGENT_SDK_CLIENT_APP", "my-app/2.3");
        let ua = get_user_agent(Some("cron"));
        assert!(ua.contains("claude-cli/"));
        assert!(ua.contains("(external, sdk"));
        assert!(ua.contains("agent-sdk/1.0.0"));
        assert!(ua.contains("client-app/my-app/2.3"));
        assert!(ua.contains("workload/cron"));
        clear_env();
    }

    #[test]
    fn user_agent_entrypoint_defaults_to_cli() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        let ua = get_user_agent(None);
        assert!(ua.contains(", cli"));
    }

    #[test]
    fn user_agent_omits_empty_workload() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        let ua = get_user_agent(None);
        assert!(!ua.contains("workload/"));
        let ua_empty = get_user_agent(Some(""));
        assert!(!ua_empty.contains("workload/"));
    }

    #[test]
    fn mcp_ua_has_no_paren_when_no_tags() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        let ua = get_mcp_user_agent();
        assert!(ua.starts_with("claude-code/"));
        assert!(!ua.contains('('));
    }

    #[test]
    fn mcp_ua_assembles_tags_when_present() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        std::env::set_var("CLAUDE_CODE_ENTRYPOINT", "cli");
        std::env::set_var("CLAUDE_AGENT_SDK_VERSION", "0.1");
        let ua = get_mcp_user_agent();
        assert!(ua.contains("(cli, agent-sdk/0.1)"));
        clear_env();
    }

    #[test]
    fn mcp_ua_single_tag_parenthesised() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        std::env::set_var("CLAUDE_CODE_ENTRYPOINT", "sdk");
        let ua = get_mcp_user_agent();
        assert!(ua.ends_with("(sdk)"));
        clear_env();
    }
}
