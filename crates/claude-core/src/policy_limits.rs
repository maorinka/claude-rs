//! Policy-limits decision layer.
//!
//! Port of the pure-decision half of `src/services/policyLimits/`. TS ships
//! ~690 LOC: ETag-cached HTTP fetch + background polling + file-backed
//! cache + disk persistence + OAuth eligibility. All of that is runtime
//! plumbing that wires to the TS axios stack; this module ports the
//! **decision** logic so callers that have a restrictions map (however
//! they loaded it) get identical answers from TS and Rust.
//!
//! The fetch / poll / persist layers are tracked separately — they need
//! the Rust ApiClient stack to grow a policy-limits endpoint first.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// One policy's restriction state. TS schema has a single `allowed: bool`
/// field; we keep the struct for future-compatibility (TS may add more).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Restriction {
    pub allowed: bool,
}

/// A full restrictions map (policy name → restriction). The absence of a
/// key means the policy is allowed (fail open).
pub type Restrictions = HashMap<String, Restriction>;

/// Policies that default to DENIED when essential-traffic-only mode is
/// active and the cache is unavailable. Without this a cache miss would
/// silently re-enable these features for HIPAA orgs. Mirrors TS
/// `ESSENTIAL_TRAFFIC_DENY_ON_MISS`.
const ESSENTIAL_TRAFFIC_DENY_ON_MISS: &[&str] = &["allow_product_feedback"];

/// Is the current session operating in essential-traffic-only mode?
/// Mirrors TS `isEssentialTrafficOnly` via the `CLAUDE_CODE_SIMPLE` env var
/// (which the TS side also honours via `isEnvTruthy` for the same flag).
fn is_essential_traffic_only() -> bool {
    std::env::var("CLAUDE_CODE_SIMPLE")
        .ok()
        .as_deref()
        .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

/// The core decision function. Returns true when the policy is allowed.
/// Unknown policies fail open. Missing cache fails open EXCEPT for the
/// essential-traffic-mode deny-list above, which fails closed.
pub fn is_policy_allowed(policy: &str, restrictions: Option<&Restrictions>) -> bool {
    match restrictions {
        None => {
            if is_essential_traffic_only() && ESSENTIAL_TRAFFIC_DENY_ON_MISS.contains(&policy) {
                return false;
            }
            true
        }
        Some(map) => match map.get(policy) {
            Some(r) => r.allowed,
            None => true, // unknown policy = allowed
        },
    }
}

// ── Process-wide session cache ─────────────────────────────────────────────

/// Holder for the restrictions map. Callers use `install_restrictions` to
/// set the policy state once it has been fetched by whatever transport
/// they provide; `is_policy_allowed_global` then consults the global
/// state, mirroring the TS singleton pattern.
#[derive(Default)]
pub struct PolicyLimitsCache {
    inner: RwLock<Option<Restrictions>>,
}

impl PolicyLimitsCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn install(&self, restrictions: Restrictions) {
        if let Ok(mut g) = self.inner.write() {
            *g = Some(restrictions);
        }
    }

    pub fn clear(&self) {
        if let Ok(mut g) = self.inner.write() {
            *g = None;
        }
    }

    pub fn is_allowed(&self, policy: &str) -> bool {
        let guard = self.inner.read().ok();
        is_policy_allowed(policy, guard.as_deref().and_then(|g| g.as_ref()))
    }
}

use std::sync::OnceLock;
static GLOBAL: OnceLock<Arc<PolicyLimitsCache>> = OnceLock::new();

pub fn global() -> Arc<PolicyLimitsCache> {
    GLOBAL
        .get_or_init(|| Arc::new(PolicyLimitsCache::new()))
        .clone()
}

/// Convenience: check against the process-wide cache.
pub fn is_policy_allowed_global(policy: &str) -> bool {
    global().is_allowed(policy)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn restriction(allowed: bool) -> Restriction {
        Restriction { allowed }
    }

    #[test]
    fn missing_cache_fails_open() {
        assert!(is_policy_allowed("anything", None));
    }

    #[test]
    fn unknown_policy_fails_open() {
        let r = Restrictions::new();
        assert!(is_policy_allowed("whatever", Some(&r)));
    }

    #[test]
    fn explicit_deny_enforced() {
        let mut r = Restrictions::new();
        r.insert("allow_product_feedback".into(), restriction(false));
        assert!(!is_policy_allowed("allow_product_feedback", Some(&r)));
    }

    #[test]
    fn explicit_allow_enforced() {
        let mut r = Restrictions::new();
        r.insert("allow_webfetch".into(), restriction(true));
        assert!(is_policy_allowed("allow_webfetch", Some(&r)));
    }

    #[test]
    fn essential_traffic_deny_on_miss_without_cache() {
        std::env::set_var("CLAUDE_CODE_SIMPLE", "1");
        assert!(!is_policy_allowed("allow_product_feedback", None));
        assert!(is_policy_allowed("allow_other", None));
        std::env::remove_var("CLAUDE_CODE_SIMPLE");
    }

    #[test]
    fn global_cache_install_and_check() {
        let c = PolicyLimitsCache::new();
        assert!(c.is_allowed("x")); // empty cache => fail open

        let mut r = Restrictions::new();
        r.insert("allow_x".into(), restriction(false));
        c.install(r);
        assert!(!c.is_allowed("allow_x"));
        assert!(c.is_allowed("unknown"));

        c.clear();
        assert!(c.is_allowed("allow_x")); // back to fail-open after clear
    }
}
