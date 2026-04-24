//! Env-var classification lists for managed-settings enforcement.
//!
//! Port of TS `utils/managedEnvConstants.ts:1-191`.
//!
//! Three distinct lists:
//! - `PROVIDER_MANAGED_ENV_VARS` / `PROVIDER_MANAGED_ENV_PREFIXES` —
//!   vars that control inference routing. Stripped from settings-sourced
//!   env when `CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST` is truthy in the
//!   spawn env.
//! - `DANGEROUS_SHELL_SETTINGS` — settings that can execute arbitrary
//!   shell code. Pre-trust enforcement blocks these.
//! - `SAFE_ENV_VARS` — allowlist for env vars safe to apply before the
//!   trust dialog. **Anything NOT here is considered dangerous** per
//!   the TS source-of-truth contract; keep in exact sync.

use once_cell::sync::Lazy;
use std::collections::HashSet;

/// Lowercase "dangerous" setting keys that can execute arbitrary shell
/// code (via hooks / helpers / status-line). Matches TS
/// `managedEnvConstants.ts:75-82`.
pub const DANGEROUS_SHELL_SETTINGS: &[&str] = &[
    "apiKeyHelper",
    "awsAuthRefresh",
    "awsCredentialExport",
    "gcpAuthRefresh",
    "otelHeadersHelper",
    "statusLine",
];

/// Prefixes that match any per-model Vertex region override. TS source
/// comment calls out that this scales with model releases, so a prefix
/// match avoids drift on each launch.
pub const PROVIDER_MANAGED_ENV_PREFIXES: &[&str] = &["VERTEX_REGION_CLAUDE_"];

/// Exact-match list — inference routing / auth / model-default vars.
/// TS stores this as a `Set<string>` and calls `.has(upper)`; the Rust
/// port uses a `Lazy<HashSet>` for the same O(1) lookup and keeps the
/// initialiser literal list readable.
static PROVIDER_MANAGED_ENV_VARS: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    [
        // The flag itself — settings can't unset it once the host set it.
        "CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST",
        // Provider selection
        "CLAUDE_CODE_USE_BEDROCK",
        "CLAUDE_CODE_USE_VERTEX",
        "CLAUDE_CODE_USE_FOUNDRY",
        // Endpoint config
        "ANTHROPIC_BASE_URL",
        "ANTHROPIC_BEDROCK_BASE_URL",
        "ANTHROPIC_VERTEX_BASE_URL",
        "ANTHROPIC_FOUNDRY_BASE_URL",
        "ANTHROPIC_FOUNDRY_RESOURCE",
        "ANTHROPIC_VERTEX_PROJECT_ID",
        // Region routing
        "CLOUD_ML_REGION",
        // Auth
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "CLAUDE_CODE_OAUTH_TOKEN",
        "AWS_BEARER_TOKEN_BEDROCK",
        "ANTHROPIC_FOUNDRY_API_KEY",
        "CLAUDE_CODE_SKIP_BEDROCK_AUTH",
        "CLAUDE_CODE_SKIP_VERTEX_AUTH",
        "CLAUDE_CODE_SKIP_FOUNDRY_AUTH",
        // Model defaults
        "ANTHROPIC_MODEL",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL_DESCRIPTION",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL_SUPPORTED_CAPABILITIES",
        "ANTHROPIC_DEFAULT_OPUS_MODEL",
        "ANTHROPIC_DEFAULT_OPUS_MODEL_DESCRIPTION",
        "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME",
        "ANTHROPIC_DEFAULT_OPUS_MODEL_SUPPORTED_CAPABILITIES",
        "ANTHROPIC_DEFAULT_SONNET_MODEL",
        "ANTHROPIC_DEFAULT_SONNET_MODEL_DESCRIPTION",
        "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME",
        "ANTHROPIC_DEFAULT_SONNET_MODEL_SUPPORTED_CAPABILITIES",
        "ANTHROPIC_SMALL_FAST_MODEL",
        "ANTHROPIC_SMALL_FAST_MODEL_AWS_REGION",
        "CLAUDE_CODE_SUBAGENT_MODEL",
    ]
    .into_iter()
    .collect()
});

/// Returns `true` if `key` matches either the exact-match set or any
/// configured prefix. Case-insensitive — TS uppercases the key before
/// checking, Rust does the same.
pub fn is_provider_managed_env_var(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    PROVIDER_MANAGED_ENV_VARS.contains(upper.as_str())
        || PROVIDER_MANAGED_ENV_PREFIXES
            .iter()
            .any(|p| upper.starts_with(p))
}

/// Safe env vars — the **allowlist** that can be applied before the
/// trust dialog. TS comment makes this the source of truth: anything
/// not in this list is treated as dangerous.
///
/// IMPORTANT: keep this in exact sync with TS
/// `managedEnvConstants.ts:108-191`. A missing entry here means a safe
/// var is treated as dangerous (usability regression); an extra entry
/// means a dangerous var is treated as safe (security regression).
pub static SAFE_ENV_VARS: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    [
        "ANTHROPIC_CUSTOM_HEADERS",
        "ANTHROPIC_CUSTOM_MODEL_OPTION",
        "ANTHROPIC_CUSTOM_MODEL_OPTION_DESCRIPTION",
        "ANTHROPIC_CUSTOM_MODEL_OPTION_NAME",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL_DESCRIPTION",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL_SUPPORTED_CAPABILITIES",
        "ANTHROPIC_DEFAULT_OPUS_MODEL",
        "ANTHROPIC_DEFAULT_OPUS_MODEL_DESCRIPTION",
        "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME",
        "ANTHROPIC_DEFAULT_OPUS_MODEL_SUPPORTED_CAPABILITIES",
        "ANTHROPIC_DEFAULT_SONNET_MODEL",
        "ANTHROPIC_DEFAULT_SONNET_MODEL_DESCRIPTION",
        "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME",
        "ANTHROPIC_DEFAULT_SONNET_MODEL_SUPPORTED_CAPABILITIES",
        "ANTHROPIC_FOUNDRY_API_KEY",
        "ANTHROPIC_MODEL",
        "ANTHROPIC_SMALL_FAST_MODEL_AWS_REGION",
        "ANTHROPIC_SMALL_FAST_MODEL",
        "AWS_DEFAULT_REGION",
        "AWS_PROFILE",
        "AWS_REGION",
        "BASH_DEFAULT_TIMEOUT_MS",
        "BASH_MAX_OUTPUT_LENGTH",
        "BASH_MAX_TIMEOUT_MS",
        "CLAUDE_BASH_MAINTAIN_PROJECT_WORKING_DIR",
        "CLAUDE_CODE_API_KEY_HELPER_TTL_MS",
        "CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS",
        "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC",
        "CLAUDE_CODE_DISABLE_TERMINAL_TITLE",
        "CLAUDE_CODE_ENABLE_TELEMETRY",
        "CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS",
        "CLAUDE_CODE_IDE_SKIP_AUTO_INSTALL",
        "CLAUDE_CODE_MAX_OUTPUT_TOKENS",
        "CLAUDE_CODE_SKIP_BEDROCK_AUTH",
        "CLAUDE_CODE_SKIP_FOUNDRY_AUTH",
        "CLAUDE_CODE_SKIP_VERTEX_AUTH",
        "CLAUDE_CODE_SUBAGENT_MODEL",
        "CLAUDE_CODE_USE_BEDROCK",
        "CLAUDE_CODE_USE_FOUNDRY",
        "CLAUDE_CODE_USE_VERTEX",
        "DISABLE_AUTOUPDATER",
        "DISABLE_BUG_COMMAND",
        "DISABLE_COST_WARNINGS",
        "DISABLE_ERROR_REPORTING",
        "DISABLE_FEEDBACK_COMMAND",
        "DISABLE_TELEMETRY",
        "ENABLE_TOOL_SEARCH",
        "MAX_MCP_OUTPUT_TOKENS",
        "MAX_THINKING_TOKENS",
        "MCP_TIMEOUT",
        "MCP_TOOL_TIMEOUT",
        "OTEL_EXPORTER_OTLP_HEADERS",
        "OTEL_EXPORTER_OTLP_LOGS_HEADERS",
        "OTEL_EXPORTER_OTLP_LOGS_PROTOCOL",
        "OTEL_EXPORTER_OTLP_METRICS_CLIENT_CERTIFICATE",
        "OTEL_EXPORTER_OTLP_METRICS_CLIENT_KEY",
        "OTEL_EXPORTER_OTLP_METRICS_HEADERS",
        "OTEL_EXPORTER_OTLP_METRICS_PROTOCOL",
        "OTEL_EXPORTER_OTLP_PROTOCOL",
        "OTEL_EXPORTER_OTLP_TRACES_HEADERS",
        "OTEL_LOG_TOOL_DETAILS",
        "OTEL_LOG_USER_PROMPTS",
        "OTEL_LOGS_EXPORT_INTERVAL",
        "OTEL_LOGS_EXPORTER",
        "OTEL_METRIC_EXPORT_INTERVAL",
        "OTEL_METRICS_EXPORTER",
        "OTEL_METRICS_INCLUDE_ACCOUNT_UUID",
        "OTEL_METRICS_INCLUDE_SESSION_ID",
        "OTEL_METRICS_INCLUDE_VERSION",
        "OTEL_RESOURCE_ATTRIBUTES",
        "USE_BUILTIN_RIPGREP",
        "VERTEX_REGION_CLAUDE_3_5_HAIKU",
        "VERTEX_REGION_CLAUDE_3_5_SONNET",
        "VERTEX_REGION_CLAUDE_3_7_SONNET",
        "VERTEX_REGION_CLAUDE_4_0_OPUS",
        "VERTEX_REGION_CLAUDE_4_0_SONNET",
        "VERTEX_REGION_CLAUDE_4_1_OPUS",
        "VERTEX_REGION_CLAUDE_4_5_SONNET",
        "VERTEX_REGION_CLAUDE_4_6_SONNET",
        "VERTEX_REGION_CLAUDE_HAIKU_4_5",
    ]
    .into_iter()
    .collect()
});

/// Case-insensitive check against [`SAFE_ENV_VARS`]. Convenience wrapper
/// so callers don't have to uppercase before lookup.
pub fn is_safe_env_var(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    SAFE_ENV_VARS.contains(upper.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dangerous_shell_settings_are_the_six_ts_entries() {
        // Pins the TS list exactly — adding / removing a setting here
        // without coordinating with the enforcement code is a security
        // drift we want CI to flag.
        assert_eq!(
            DANGEROUS_SHELL_SETTINGS,
            &[
                "apiKeyHelper",
                "awsAuthRefresh",
                "awsCredentialExport",
                "gcpAuthRefresh",
                "otelHeadersHelper",
                "statusLine",
            ]
        );
    }

    #[test]
    fn is_provider_managed_exact_match() {
        assert!(is_provider_managed_env_var("ANTHROPIC_API_KEY"));
        assert!(is_provider_managed_env_var("CLAUDE_CODE_USE_BEDROCK"));
        assert!(is_provider_managed_env_var("ANTHROPIC_MODEL"));
    }

    #[test]
    fn is_provider_managed_case_insensitive() {
        // TS uppercases before the check; Rust must too.
        assert!(is_provider_managed_env_var("anthropic_api_key"));
        assert!(is_provider_managed_env_var("Anthropic_Api_Key"));
    }

    #[test]
    fn is_provider_managed_prefix_match() {
        assert!(is_provider_managed_env_var(
            "VERTEX_REGION_CLAUDE_4_5_SONNET"
        ));
        assert!(is_provider_managed_env_var("VERTEX_REGION_CLAUDE_foo_bar"));
        // Prefix-only: the bare prefix (= exact string) also matches.
        assert!(is_provider_managed_env_var("VERTEX_REGION_CLAUDE_"));
    }

    #[test]
    fn is_provider_managed_rejects_unrelated() {
        assert!(!is_provider_managed_env_var("HOME"));
        assert!(!is_provider_managed_env_var("PATH"));
        assert!(!is_provider_managed_env_var("NODE_TLS_REJECT_UNAUTHORIZED"));
        // Superstring of a managed name — `_TEST` suffix means it's NOT
        // the exact managed var and not a prefix match.
        assert!(!is_provider_managed_env_var("ANTHROPIC_API_KEY_TEST"));
    }

    #[test]
    fn safe_env_vars_contains_known_entries() {
        assert!(is_safe_env_var("ANTHROPIC_MODEL"));
        assert!(is_safe_env_var("BASH_MAX_OUTPUT_LENGTH"));
        assert!(is_safe_env_var("OTEL_LOGS_EXPORTER"));
        assert!(is_safe_env_var("USE_BUILTIN_RIPGREP"));
    }

    #[test]
    fn safe_env_vars_rejects_dangerous() {
        // The dangerous vars the TS comment explicitly calls out must
        // not appear in the allowlist.
        assert!(!is_safe_env_var("ANTHROPIC_BASE_URL"));
        assert!(!is_safe_env_var("HTTPS_PROXY"));
        assert!(!is_safe_env_var("NODE_EXTRA_CA_CERTS"));
        assert!(!is_safe_env_var("NODE_TLS_REJECT_UNAUTHORIZED"));
        assert!(!is_safe_env_var("ANTHROPIC_API_KEY"));
    }

    #[test]
    fn safe_env_vars_case_insensitive_lookup() {
        assert!(is_safe_env_var("anthropic_model"));
        assert!(is_safe_env_var("Anthropic_Model"));
    }

    #[test]
    fn safe_env_vars_count_matches_ts() {
        // Catches accidental additions/removals — TS has 82 entries.
        assert_eq!(SAFE_ENV_VARS.len(), 82);
    }

    #[test]
    fn provider_managed_vars_count_matches_ts() {
        // TS has 35 exact-match entries. A mismatch here means the port
        // fell out of sync and settings.env stripping is incorrect.
        assert_eq!(PROVIDER_MANAGED_ENV_VARS.len(), 35);
    }

    #[test]
    fn the_managed_flag_itself_is_managed() {
        // Security contract: settings can't unset the flag once the host
        // sets it (TS comment at constants.ts:16). Classifying the flag
        // itself as provider-managed is what makes that stripping work.
        assert!(is_provider_managed_env_var(
            "CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST"
        ));
    }
}
