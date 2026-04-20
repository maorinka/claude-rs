//! Thinking-mode configuration for a query.
//!
//! Port of TS `utils/thinking.ts:10-13` (`ThinkingConfig` only;
//! function bodies depend on settings / GrowthBook / model-provider
//! subsystems that aren't ported).
//!
//! Variants
//! ========
//! - `Adaptive` — model decides when to think (default for
//!   thinking-supported providers).
//! - `Enabled { budget_tokens }` — explicit budget cap on the
//!   reasoning phase.
//! - `Disabled` — thinking suppressed entirely.
//!
//! Serialises to the TS wire format: `{"type": "adaptive"}`,
//! `{"type": "enabled", "budgetTokens": 8000}`,
//! `{"type": "disabled"}`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ThinkingConfig {
    /// `{type: 'adaptive'}` — model decides when to think.
    Adaptive,
    /// `{type: 'enabled', budgetTokens: N}` — explicit token cap on
    /// the reasoning phase.
    Enabled {
        #[serde(rename = "budgetTokens")]
        budget_tokens: u32,
    },
    /// `{type: 'disabled'}` — thinking suppressed.
    Disabled,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn adaptive_roundtrips() {
        let cfg = ThinkingConfig::Adaptive;
        let v = serde_json::to_value(&cfg).unwrap();
        assert_eq!(v, json!({ "type": "adaptive" }));
        let back: ThinkingConfig = serde_json::from_value(v).unwrap();
        assert_eq!(back, cfg);
    }

    #[test]
    fn enabled_roundtrips_with_budget_camel_case() {
        let cfg = ThinkingConfig::Enabled { budget_tokens: 8000 };
        let v = serde_json::to_value(&cfg).unwrap();
        // TS uses camelCase `budgetTokens`; Rust rename to match.
        assert_eq!(v, json!({ "type": "enabled", "budgetTokens": 8000 }));
        let back: ThinkingConfig = serde_json::from_value(v).unwrap();
        assert_eq!(back, cfg);
    }

    #[test]
    fn disabled_roundtrips() {
        let cfg = ThinkingConfig::Disabled;
        let v = serde_json::to_value(&cfg).unwrap();
        assert_eq!(v, json!({ "type": "disabled" }));
        let back: ThinkingConfig = serde_json::from_value(v).unwrap();
        assert_eq!(back, cfg);
    }

    #[test]
    fn rejects_unknown_variant() {
        let v = json!({ "type": "other" });
        assert!(serde_json::from_value::<ThinkingConfig>(v).is_err());
    }
}
