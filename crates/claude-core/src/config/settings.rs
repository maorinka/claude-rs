use crate::sandbox::types::SandboxSettings;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;

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
    #[serde(rename = "defaultMode", skip_serializing_if = "Option::is_none")]
    pub default_mode: Option<String>,
}

/// Configuration for a single MCP server entry in settings.json.
///
/// Matches the TS `McpServerConfig` union. `type` defaults to stdio when it is
/// omitted, preserving the older settings shape:
/// ```json
/// {
///   "command": "npx",
///   "args": ["-y", "@some/mcp-server"],
///   "env": {}
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct McpServerSettingsEntry {
    #[serde(rename = "type")]
    pub transport_type: Option<String>,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(rename = "authToken", default)]
    pub auth_token: Option<String>,
}

impl McpServerSettingsEntry {
    pub fn to_mcp_server_config(
        &self,
    ) -> Result<crate::mcp::types::McpServerConfig, serde_json::Error> {
        let mut value = serde_json::to_value(self)?;
        if let Some(obj) = value.as_object_mut() {
            if self.transport_type.is_none() {
                obj.remove("type");
            }
            obj.retain(|_, value| match value {
                serde_json::Value::Null => false,
                serde_json::Value::String(text) => !text.is_empty(),
                serde_json::Value::Array(items) => !items.is_empty(),
                serde_json::Value::Object(map) => !map.is_empty(),
                _ => true,
            });
        }
        crate::mcp::types::McpServerConfig::from_value(value)
    }
}

/// XAA IdP connection settings. Mirrors TS `settings.xaaIdp`; secrets and
/// id_tokens live in secure storage under issuer-keyed maps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct XaaIdpSettings {
    pub issuer: String,
    pub client_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub callback_port: Option<u32>,
}

impl Default for XaaIdpSettings {
    fn default() -> Self {
        Self {
            issuer: String::new(),
            client_id: String::new(),
            callback_port: None,
        }
    }
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

    /// Thinking effort level (`low`, `medium`, `high`, `max`, `auto`).
    #[serde(rename = "effortLevel", skip_serializing_if = "Option::is_none")]
    pub effort_level: Option<String>,

    /// API key override (overrides the environment variable).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,

    /// Command used to resolve an API key dynamically.
    #[serde(rename = "apiKeyHelper", skip_serializing_if = "Option::is_none")]
    pub api_key_helper: Option<String>,

    pub permissions: SettingsPermissions,

    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,

    #[serde(
        rename = "enableAllProjectMcpServers",
        skip_serializing_if = "Option::is_none"
    )]
    pub enable_all_project_mcp_servers: Option<bool>,

    #[serde(
        rename = "enabledMcpjsonServers",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub enabled_mcpjson_servers: Vec<String>,

    #[serde(
        rename = "disabledMcpjsonServers",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub disabled_mcpjson_servers: Vec<String>,

    #[serde(
        rename = "skipAutoPermissionPrompt",
        skip_serializing_if = "Option::is_none"
    )]
    pub skip_auto_permission_prompt: Option<bool>,

    /// MCP server configurations keyed by server name.
    ///
    /// Matches the TS `mcpServers` key in `~/.claude/settings.json`.
    #[serde(
        default,
        rename = "mcpServers",
        skip_serializing_if = "HashMap::is_empty"
    )]
    pub mcp_servers: HashMap<String, McpServerSettingsEntry>,

    /// Enterprise XAA IdP connection. TS gates this field from public docs/types
    /// behind `CLAUDE_CODE_ENABLE_XAA`, but the serialized key is still
    /// `xaaIdp` when configured.
    #[serde(rename = "xaaIdp", skip_serializing_if = "Option::is_none")]
    pub xaa_idp: Option<XaaIdpSettings>,

    /// Sandbox configuration for isolated bash command execution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<SandboxSettings>,

    /// Active output style name. Matches a file basename under
    /// `~/.claude/output-styles/` or `<project>/.claude/output-styles/`.
    /// Mirrors TS `outputStyle.name` setting.
    #[serde(rename = "outputStyle", skip_serializing_if = "Option::is_none")]
    pub output_style: Option<String>,

    /// Language preference (e.g. `"Japanese"`, `"French"`). Injected into
    /// the system prompt via `build_language_section(...)`. Mirrors TS
    /// `languagePreference` setting.
    #[serde(rename = "languagePreference", skip_serializing_if = "Option::is_none")]
    pub language_preference: Option<String>,

    /// Allowlist of URL patterns HTTP hooks may target. `None` means no
    /// allowlist restriction; `Some([])` blocks all HTTP hooks.
    #[serde(
        rename = "allowedHttpHookUrls",
        skip_serializing_if = "Option::is_none"
    )]
    pub allowed_http_hook_urls: Option<Vec<String>>,

    /// Policy-level env var allowlist for HTTP hook header interpolation.
    /// Intersected with each hook's own `allowedEnvVars`.
    #[serde(
        rename = "httpHookAllowedEnvVars",
        skip_serializing_if = "Option::is_none"
    )]
    pub http_hook_allowed_env_vars: Option<Vec<String>>,

    /// Agent type for the current session. Matches TS `agent` setting.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,

    /// Skip WebFetch domain-info preflight checks. Matches TS
    /// `skipWebFetchPreflight`.
    #[serde(
        rename = "skipWebFetchPreflight",
        skip_serializing_if = "Option::is_none"
    )]
    pub skip_web_fetch_preflight: Option<bool>,

    /// Preserve settings keys Rust does not understand yet.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Settings {
    /// Load settings from a JSON file, returning `Default` if the file is missing
    /// or unparseable.
    pub fn load_from_file(path: &std::path::Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Merge `overlay` on top of `self`. Fields that are `Some` in `overlay`
    /// win; fields that are `None` in `overlay` fall back to `self`.
    /// For `permissions`, the overlay's allow/deny lists replace self's when
    /// they are non-empty, otherwise self's are kept.
    /// For `mcp_servers`, the overlay's map is merged on top of self's.
    pub fn merge(&self, overlay: &Settings) -> Settings {
        fn merge_opt_vecs(
            base: &Option<Vec<String>>,
            overlay: &Option<Vec<String>>,
        ) -> Option<Vec<String>> {
            match (base, overlay) {
                (None, None) => None,
                (Some(base), None) => Some(base.clone()),
                (None, Some(overlay)) => Some(overlay.clone()),
                (Some(base), Some(overlay)) => {
                    let mut merged = base.clone();
                    merged.extend(overlay.iter().cloned());
                    Some(merged)
                }
            }
        }

        let mut merged_mcp = self.mcp_servers.clone();
        for (k, v) in &overlay.mcp_servers {
            merged_mcp.insert(k.clone(), v.clone());
        }
        Settings {
            model: overlay.model.clone().or_else(|| self.model.clone()),
            verbose: overlay.verbose.or(self.verbose),
            max_tokens: overlay.max_tokens.or(self.max_tokens),
            effort_level: overlay
                .effort_level
                .clone()
                .or_else(|| self.effort_level.clone()),
            api_key: overlay.api_key.clone().or_else(|| self.api_key.clone()),
            api_key_helper: overlay
                .api_key_helper
                .clone()
                .or_else(|| self.api_key_helper.clone()),
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
                default_mode: overlay
                    .permissions
                    .default_mode
                    .clone()
                    .or_else(|| self.permissions.default_mode.clone()),
            },
            env: {
                let mut env = self.env.clone();
                env.extend(overlay.env.clone());
                env
            },
            enable_all_project_mcp_servers: overlay
                .enable_all_project_mcp_servers
                .or(self.enable_all_project_mcp_servers),
            enabled_mcpjson_servers: if overlay.enabled_mcpjson_servers.is_empty() {
                self.enabled_mcpjson_servers.clone()
            } else {
                overlay.enabled_mcpjson_servers.clone()
            },
            disabled_mcpjson_servers: if overlay.disabled_mcpjson_servers.is_empty() {
                self.disabled_mcpjson_servers.clone()
            } else {
                overlay.disabled_mcpjson_servers.clone()
            },
            skip_auto_permission_prompt: overlay
                .skip_auto_permission_prompt
                .or(self.skip_auto_permission_prompt),
            mcp_servers: merged_mcp,
            sandbox: overlay.sandbox.clone().or_else(|| self.sandbox.clone()),
            output_style: overlay
                .output_style
                .clone()
                .or_else(|| self.output_style.clone()),
            language_preference: overlay
                .language_preference
                .clone()
                .or_else(|| self.language_preference.clone()),
            allowed_http_hook_urls: merge_opt_vecs(
                &self.allowed_http_hook_urls,
                &overlay.allowed_http_hook_urls,
            ),
            http_hook_allowed_env_vars: merge_opt_vecs(
                &self.http_hook_allowed_env_vars,
                &overlay.http_hook_allowed_env_vars,
            ),
            agent: overlay.agent.clone().or_else(|| self.agent.clone()),
            skip_web_fetch_preflight: overlay
                .skip_web_fetch_preflight
                .or(self.skip_web_fetch_preflight),
            xaa_idp: overlay.xaa_idp.clone().or_else(|| self.xaa_idp.clone()),
            extra: {
                let mut extra = self.extra.clone();
                extra.extend(overlay.extra.clone());
                extra
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skip_web_fetch_preflight_uses_ts_camel_case_key() {
        let settings: Settings = serde_json::from_str(r#"{"skipWebFetchPreflight":true}"#).unwrap();
        assert_eq!(settings.skip_web_fetch_preflight, Some(true));

        let serialized = serde_json::to_string(&settings).unwrap();
        assert!(serialized.contains("skipWebFetchPreflight"));
    }

    #[test]
    fn skip_web_fetch_preflight_merges_like_other_scalar_settings() {
        let base = Settings {
            skip_web_fetch_preflight: Some(true),
            ..Default::default()
        };
        let overlay = Settings::default();
        assert_eq!(base.merge(&overlay).skip_web_fetch_preflight, Some(true));

        let overlay = Settings {
            skip_web_fetch_preflight: Some(false),
            ..Default::default()
        };
        assert_eq!(base.merge(&overlay).skip_web_fetch_preflight, Some(false));
    }

    #[test]
    fn xaa_idp_uses_ts_camel_case_key_and_merges_as_scalar() {
        let settings: Settings = serde_json::from_str(
            r#"{"xaaIdp":{"issuer":"https://idp.example.com","clientId":"client","callbackPort":8080}}"#,
        )
        .unwrap();
        let idp = settings.xaa_idp.as_ref().unwrap();
        assert_eq!(idp.issuer, "https://idp.example.com");
        assert_eq!(idp.client_id, "client");
        assert_eq!(idp.callback_port, Some(8080));

        let serialized = serde_json::to_string(&settings).unwrap();
        assert!(serialized.contains("xaaIdp"));
        assert!(serialized.contains("clientId"));
        assert!(serialized.contains("callbackPort"));

        let base = Settings {
            xaa_idp: Some(XaaIdpSettings {
                issuer: "https://old.example.com".into(),
                client_id: "old".into(),
                callback_port: None,
            }),
            ..Default::default()
        };
        assert_eq!(
            base.merge(&Settings::default())
                .xaa_idp
                .as_ref()
                .unwrap()
                .issuer,
            "https://old.example.com"
        );

        let overlay = Settings {
            xaa_idp: Some(XaaIdpSettings {
                issuer: "https://new.example.com".into(),
                client_id: "new".into(),
                callback_port: Some(9000),
            }),
            ..Default::default()
        };
        assert_eq!(
            base.merge(&overlay).xaa_idp.as_ref().unwrap().client_id,
            "new"
        );
    }
}
