//! Privacy-level resolution + API-provider detection.
//!
//! Port of `src/utils/privacyLevel.ts` + `src/utils/model/providers.ts`.
//! Both read env vars to decide how much network traffic Claude Code
//! may generate and which backend it's talking to.

/// Privacy level ordered by restrictiveness: `Default` < `NoTelemetry`
/// < `EssentialTraffic`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrivacyLevel {
    /// Everything enabled.
    Default,
    /// Analytics/telemetry disabled (Datadog, 1P events, feedback survey).
    NoTelemetry,
    /// ALL nonessential network traffic disabled (telemetry +
    /// auto-updates, grove, release notes, model capabilities, etc.).
    EssentialTraffic,
}

impl PrivacyLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            PrivacyLevel::Default => "default",
            PrivacyLevel::NoTelemetry => "no-telemetry",
            PrivacyLevel::EssentialTraffic => "essential-traffic",
        }
    }
}

/// Resolve the current privacy level. Env precedence matches TS:
/// `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC` wins over
/// `DISABLE_TELEMETRY`.
pub fn get_privacy_level() -> PrivacyLevel {
    if std::env::var("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC").is_ok() {
        return PrivacyLevel::EssentialTraffic;
    }
    if std::env::var("DISABLE_TELEMETRY").is_ok() {
        return PrivacyLevel::NoTelemetry;
    }
    PrivacyLevel::Default
}

/// True when all nonessential network traffic should be suppressed.
pub fn is_essential_traffic_only() -> bool {
    matches!(get_privacy_level(), PrivacyLevel::EssentialTraffic)
}

/// True when telemetry/analytics should be suppressed. True at both
/// `NoTelemetry` and `EssentialTraffic` levels.
pub fn is_telemetry_disabled() -> bool {
    !matches!(get_privacy_level(), PrivacyLevel::Default)
}

/// Name of the env var responsible for the current essential-traffic
/// restriction, or `None`. Used for "unset X to re-enable" messages.
pub fn get_essential_traffic_only_reason() -> Option<&'static str> {
    if std::env::var("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC").is_ok() {
        return Some("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC");
    }
    None
}

// ── API provider detection ────────────────────────────────────────────────

use crate::errors_util::is_env_truthy;

/// Which backend Claude Code is pointed at. Controls header sets,
/// retry policies, beta filtering, and beta-on-countTokens gating.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ApiProvider {
    FirstParty,
    Bedrock,
    Vertex,
    Foundry,
}

impl ApiProvider {
    pub fn as_str(&self) -> &'static str {
        match self {
            ApiProvider::FirstParty => "firstParty",
            ApiProvider::Bedrock => "bedrock",
            ApiProvider::Vertex => "vertex",
            ApiProvider::Foundry => "foundry",
        }
    }
}

/// Resolve the current API provider from the standard CLAUDE_CODE_USE_*
/// env vars. First-party wins when no flag is set.
pub fn get_api_provider() -> ApiProvider {
    if is_env_truthy("CLAUDE_CODE_USE_BEDROCK") {
        return ApiProvider::Bedrock;
    }
    if is_env_truthy("CLAUDE_CODE_USE_VERTEX") {
        return ApiProvider::Vertex;
    }
    if is_env_truthy("CLAUDE_CODE_USE_FOUNDRY") {
        return ApiProvider::Foundry;
    }
    ApiProvider::FirstParty
}

/// Check if `ANTHROPIC_BASE_URL` is a first-party Anthropic API URL.
/// Returns true when unset (default), when it points at
/// `api.anthropic.com`, or when it points at
/// `api-staging.anthropic.com` for ant users.
pub fn is_first_party_anthropic_base_url() -> bool {
    let Ok(raw) = std::env::var("ANTHROPIC_BASE_URL") else {
        return true;
    };
    let Ok(url) = url::Url::parse(&raw) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    if host == "api.anthropic.com" {
        return true;
    }
    if crate::user_type::is_ant() && host == "api-staging.anthropic.com" {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn clear_privacy_env() {
        std::env::remove_var("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC");
        std::env::remove_var("DISABLE_TELEMETRY");
    }

    fn clear_provider_env() {
        std::env::remove_var("CLAUDE_CODE_USE_BEDROCK");
        std::env::remove_var("CLAUDE_CODE_USE_VERTEX");
        std::env::remove_var("CLAUDE_CODE_USE_FOUNDRY");
        std::env::remove_var("ANTHROPIC_BASE_URL");
        std::env::remove_var("USER_TYPE");
    }

    #[test]
    fn privacy_default_when_no_env() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_privacy_env();
        assert_eq!(get_privacy_level(), PrivacyLevel::Default);
        assert!(!is_essential_traffic_only());
        assert!(!is_telemetry_disabled());
        assert!(get_essential_traffic_only_reason().is_none());
    }

    #[test]
    fn privacy_disable_telemetry_triggers_no_telemetry() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_privacy_env();
        std::env::set_var("DISABLE_TELEMETRY", "1");
        assert_eq!(get_privacy_level(), PrivacyLevel::NoTelemetry);
        assert!(is_telemetry_disabled());
        assert!(!is_essential_traffic_only());
        clear_privacy_env();
    }

    #[test]
    fn privacy_essential_wins_over_telemetry() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_privacy_env();
        std::env::set_var("DISABLE_TELEMETRY", "1");
        std::env::set_var("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1");
        assert_eq!(get_privacy_level(), PrivacyLevel::EssentialTraffic);
        assert!(is_essential_traffic_only());
        assert!(is_telemetry_disabled());
        assert_eq!(
            get_essential_traffic_only_reason(),
            Some("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC")
        );
        clear_privacy_env();
    }

    #[test]
    fn provider_first_party_is_default() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_provider_env();
        assert_eq!(get_api_provider(), ApiProvider::FirstParty);
    }

    #[test]
    fn provider_bedrock_wins_when_set() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_provider_env();
        std::env::set_var("CLAUDE_CODE_USE_BEDROCK", "1");
        std::env::set_var("CLAUDE_CODE_USE_VERTEX", "1");
        assert_eq!(get_api_provider(), ApiProvider::Bedrock);
        clear_provider_env();
    }

    #[test]
    fn provider_vertex_after_bedrock() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_provider_env();
        std::env::set_var("CLAUDE_CODE_USE_VERTEX", "true");
        assert_eq!(get_api_provider(), ApiProvider::Vertex);
        clear_provider_env();
    }

    #[test]
    fn first_party_base_url_default_unset() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_provider_env();
        assert!(is_first_party_anthropic_base_url());
    }

    #[test]
    fn first_party_base_url_anthropic_host() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_provider_env();
        std::env::set_var("ANTHROPIC_BASE_URL", "https://api.anthropic.com");
        assert!(is_first_party_anthropic_base_url());
        clear_provider_env();
    }

    #[test]
    fn first_party_base_url_staging_only_for_ant() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_provider_env();
        std::env::set_var(
            "ANTHROPIC_BASE_URL",
            "https://api-staging.anthropic.com",
        );
        assert!(!is_first_party_anthropic_base_url());
        std::env::set_var("USER_TYPE", "ant");
        assert!(is_first_party_anthropic_base_url());
        clear_provider_env();
    }

    #[test]
    fn first_party_base_url_rejects_unknown_host() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_provider_env();
        std::env::set_var("ANTHROPIC_BASE_URL", "https://evil.example/api");
        assert!(!is_first_party_anthropic_base_url());
        clear_provider_env();
    }

    #[test]
    fn first_party_base_url_malformed_is_not_first_party() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_provider_env();
        std::env::set_var("ANTHROPIC_BASE_URL", "not a valid url");
        assert!(!is_first_party_anthropic_base_url());
        clear_provider_env();
    }

    #[test]
    fn as_str_round_trips_for_telemetry_gauge() {
        assert_eq!(PrivacyLevel::Default.as_str(), "default");
        assert_eq!(PrivacyLevel::NoTelemetry.as_str(), "no-telemetry");
        assert_eq!(
            PrivacyLevel::EssentialTraffic.as_str(),
            "essential-traffic"
        );
        assert_eq!(ApiProvider::FirstParty.as_str(), "firstParty");
        assert_eq!(ApiProvider::Bedrock.as_str(), "bedrock");
    }
}
