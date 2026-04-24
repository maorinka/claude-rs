//! Secret-scrubbed subprocess env for GitHub Actions workflows.
//!
//! Port of TS `utils/subprocessEnv.ts:1-99`.
//!
//! When running inside a claude-code-action workflow that exposes the
//! session to untrusted content (prompt injection surface — set via
//! `allowed_non_write_users`), the action exports
//! `CLAUDE_CODE_SUBPROCESS_ENV_SCRUB=1`. This module honours the flag
//! by stripping ~20 sensitive vars from the subprocess environment so
//! `${ANTHROPIC_API_KEY}`-style shell expansion in Bash-tool commands
//! can't exfiltrate them.
//!
//! The parent claude process keeps the vars (needed for API calls and
//! lazy credential reads). Only children (Bash, MCP stdio, LSP, hooks)
//! are scrubbed.

use crate::errors_util::is_env_truthy;
use std::collections::HashMap;

/// Vars to strip from subprocess env when the GHA scrub flag is set.
/// Keep in exact sync with TS `GHA_SUBPROCESS_SCRUB` — drift here is a
/// security regression.
pub const GHA_SUBPROCESS_SCRUB: &[&str] = &[
    // Anthropic auth — lazy in-process reads, subprocesses don't need these.
    "ANTHROPIC_API_KEY",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_FOUNDRY_API_KEY",
    "ANTHROPIC_CUSTOM_HEADERS",
    // OTLP exporter headers — documented to carry `Authorization: Bearer …`.
    "OTEL_EXPORTER_OTLP_HEADERS",
    "OTEL_EXPORTER_OTLP_LOGS_HEADERS",
    "OTEL_EXPORTER_OTLP_METRICS_HEADERS",
    "OTEL_EXPORTER_OTLP_TRACES_HEADERS",
    // Cloud provider creds (lazy SDK reads).
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "AWS_BEARER_TOKEN_BEDROCK",
    "GOOGLE_APPLICATION_CREDENTIALS",
    "AZURE_CLIENT_SECRET",
    "AZURE_CLIENT_CERTIFICATE_PATH",
    // GitHub Actions OIDC — consumed by action JS before claude spawns;
    // leaking these allows minting an App installation token → repo takeover.
    "ACTIONS_ID_TOKEN_REQUEST_TOKEN",
    "ACTIONS_ID_TOKEN_REQUEST_URL",
    // GitHub Actions artifact / cache — cache poisoning → supply-chain pivot.
    "ACTIONS_RUNTIME_TOKEN",
    "ACTIONS_RUNTIME_URL",
    // claude-code-action duplicates — `ALL_INPUTS` contains the API key as JSON.
    "ALL_INPUTS",
    "OVERRIDE_GITHUB_TOKEN",
    "DEFAULT_WORKFLOW_TOKEN",
    "SSH_SIGNING_KEY",
];

const SCRUB_ENV_FLAG: &str = "CLAUDE_CODE_SUBPROCESS_ENV_SCRUB";

/// Returns a copy of the current process env with GHA secrets stripped
/// iff `CLAUDE_CODE_SUBPROCESS_ENV_SCRUB` is truthy. Optionally merges
/// additional vars (upstream-proxy CA bundle, `HTTPS_PROXY`, etc.) —
/// TS passes these via the `_getUpstreamProxyEnv` hook; Rust takes
/// them directly.
///
/// Also strips the `INPUT_<NAME>` GitHub Actions mirror of each
/// scrubbed var, since the runner auto-exports those for `with:`
/// inputs.
pub fn subprocess_env(extra: &HashMap<String, String>) -> HashMap<String, String> {
    let mut env: HashMap<String, String> = std::env::vars().collect();
    // Extra always applied (non-CCR sessions pass `{}` which is a no-op).
    for (k, v) in extra {
        env.insert(k.clone(), v.clone());
    }

    if !is_env_truthy(SCRUB_ENV_FLAG) {
        return env;
    }

    for k in GHA_SUBPROCESS_SCRUB {
        env.remove(*k);
        let mirror = format!("INPUT_{k}");
        env.remove(&mirror);
    }
    env
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn lock_env() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner())
    }

    #[test]
    fn scrub_list_length_pinned() {
        // 23 entries in the TS source — a drift means a new secret was
        // added to either side and the port missed it.
        assert_eq!(GHA_SUBPROCESS_SCRUB.len(), 23);
    }

    #[test]
    fn scrub_includes_critical_names() {
        for k in [
            "ANTHROPIC_API_KEY",
            "CLAUDE_CODE_OAUTH_TOKEN",
            "ACTIONS_ID_TOKEN_REQUEST_TOKEN",
            "ALL_INPUTS",
        ] {
            assert!(
                GHA_SUBPROCESS_SCRUB.contains(&k),
                "expected {k} in scrub list"
            );
        }
    }

    #[test]
    fn scrub_excludes_github_token() {
        // TS comment explicitly keeps `GITHUB_TOKEN` and `GH_TOKEN` —
        // wrapper scripts (gh.sh) need them. Guard against accidental
        // inclusion.
        assert!(!GHA_SUBPROCESS_SCRUB.contains(&"GITHUB_TOKEN"));
        assert!(!GHA_SUBPROCESS_SCRUB.contains(&"GH_TOKEN"));
    }

    #[test]
    fn without_flag_returns_parent_env_with_extras() {
        let _g = lock_env();
        std::env::remove_var(SCRUB_ENV_FLAG);
        std::env::set_var("ANTHROPIC_API_KEY", "test-key");
        let mut extra = HashMap::new();
        extra.insert("HTTPS_PROXY".to_string(), "http://proxy".to_string());

        let env = subprocess_env(&extra);
        assert_eq!(
            env.get("ANTHROPIC_API_KEY").map(String::as_str),
            Some("test-key")
        );
        assert_eq!(
            env.get("HTTPS_PROXY").map(String::as_str),
            Some("http://proxy")
        );

        std::env::remove_var("ANTHROPIC_API_KEY");
    }

    #[test]
    fn with_flag_strips_scrubbed_vars() {
        let _g = lock_env();
        std::env::set_var(SCRUB_ENV_FLAG, "1");
        std::env::set_var("ANTHROPIC_API_KEY", "secret");
        std::env::set_var("INPUT_ANTHROPIC_API_KEY", "secret-from-inputs");
        std::env::set_var("ACTIONS_RUNTIME_TOKEN", "runtime-token");

        let env = subprocess_env(&HashMap::new());
        assert!(!env.contains_key("ANTHROPIC_API_KEY"));
        assert!(!env.contains_key("INPUT_ANTHROPIC_API_KEY"));
        assert!(!env.contains_key("ACTIONS_RUNTIME_TOKEN"));

        std::env::remove_var(SCRUB_ENV_FLAG);
        std::env::remove_var("ANTHROPIC_API_KEY");
        std::env::remove_var("INPUT_ANTHROPIC_API_KEY");
        std::env::remove_var("ACTIONS_RUNTIME_TOKEN");
    }

    #[test]
    fn scrub_preserves_non_sensitive_vars() {
        let _g = lock_env();
        std::env::set_var(SCRUB_ENV_FLAG, "1");
        std::env::set_var("PATH_TEST_SENTINEL_VAR", "intact");
        std::env::set_var("GITHUB_TOKEN", "keeper");

        let env = subprocess_env(&HashMap::new());
        assert_eq!(
            env.get("PATH_TEST_SENTINEL_VAR").map(String::as_str),
            Some("intact")
        );
        assert_eq!(env.get("GITHUB_TOKEN").map(String::as_str), Some("keeper"));

        std::env::remove_var(SCRUB_ENV_FLAG);
        std::env::remove_var("PATH_TEST_SENTINEL_VAR");
        std::env::remove_var("GITHUB_TOKEN");
    }

    #[test]
    fn extras_merge_after_scrub_and_override() {
        let _g = lock_env();
        std::env::set_var(SCRUB_ENV_FLAG, "1");
        let mut extra = HashMap::new();
        extra.insert("HTTPS_PROXY".to_string(), "http://relay:8080".to_string());
        // Even if the extra map would re-introduce a scrubbed var, the
        // scrub runs AFTER the merge — but TS code does the merge
        // before the scrub too. Assert parity:
        extra.insert(
            "ANTHROPIC_API_KEY".to_string(),
            "should-be-removed".to_string(),
        );

        let env = subprocess_env(&extra);
        assert_eq!(
            env.get("HTTPS_PROXY").map(String::as_str),
            Some("http://relay:8080")
        );
        // Extras that collide with the scrub list STILL get scrubbed —
        // matches TS behaviour (scrub runs on the merged object).
        assert!(!env.contains_key("ANTHROPIC_API_KEY"));

        std::env::remove_var(SCRUB_ENV_FLAG);
    }
}
