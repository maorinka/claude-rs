use serde::{Deserialize, Serialize};

/// A single permission rule referencing a tool, with an optional glob pattern.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PermissionRuleConfig {
    pub tool: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
}

/// Allow/deny lists of permission rules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SettingsPermissions {
    pub allow: Vec<PermissionRuleConfig>,
    pub deny: Vec<PermissionRuleConfig>,
}

/// Top-level settings structure. All fields are optional so that partial
/// configurations can be layered via `merge`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Settings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub verbose: Option<bool>,

    /// Maximum tokens for the model response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,

    /// API key override (overrides the environment variable).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,

    pub permissions: SettingsPermissions,
}

impl Settings {
    /// Merge `overlay` on top of `self`. Fields that are `Some` in `overlay`
    /// win; fields that are `None` in `overlay` fall back to `self`.
    /// For `permissions`, the overlay's allow/deny lists replace self's when
    /// they are non-empty, otherwise self's are kept.
    pub fn merge(&self, overlay: &Settings) -> Settings {
        Settings {
            model: overlay.model.clone().or_else(|| self.model.clone()),
            verbose: overlay.verbose.or(self.verbose),
            max_tokens: overlay.max_tokens.or(self.max_tokens),
            api_key: overlay.api_key.clone().or_else(|| self.api_key.clone()),
            permissions: SettingsPermissions {
                allow: if overlay.permissions.allow.is_empty() {
                    self.permissions.allow.clone()
                } else {
                    overlay.permissions.allow.clone()
                },
                deny: if overlay.permissions.deny.is_empty() {
                    self.permissions.deny.clone()
                } else {
                    overlay.permissions.deny.clone()
                },
            },
        }
    }
}
