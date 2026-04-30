//! Config → settings migrations.
//!
//! Ports TS config/settings migrations that move legacy global/project fields
//! into the settings shape.

use super::MigrationContext;
use crate::config::global::GlobalConfig;
use crate::config::settings::Settings;
use serde_json::Value;

pub fn migrate_auto_updates_to_settings(
    global: &mut GlobalConfig,
    settings: &mut Settings,
) -> bool {
    if global.extra.get("autoUpdates") != Some(&Value::Bool(false))
        || global.extra.get("autoUpdatesProtectedForNative") == Some(&Value::Bool(true))
    {
        return false;
    }
    settings
        .env
        .insert("DISABLE_AUTOUPDATER".into(), "1".into());
    global.extra.remove("autoUpdates");
    global.extra.remove("autoUpdatesProtectedForNative");
    true
}

/// Port of `migrateBypassPermissionsAcceptedToSettings`. If the legacy
/// `bypassPermissionsModeAccepted` flag is set in global config, copy it
/// across to `skipDangerousModePermissionPrompt` in user settings (stored
/// under the Settings `extra` map on the Rust side — actually, Settings
/// doesn't have one. Stored in-config only for now if the main structs
/// don't surface the field). Returns true when state changes.
pub fn migrate_bypass_permissions(global: &mut GlobalConfig, _settings: &mut Settings) -> bool {
    match global.extra.get("bypassPermissionsModeAccepted") {
        Some(Value::Bool(true)) => {
            // Record the migrated state in global.extra under the new name,
            // so a later read of merged settings will pick it up. The TS
            // code writes to userSettings — our Settings type doesn't have
            // a matching field yet, so we mirror the value into global.extra
            // as a waypoint. Removing the old key makes the migration
            // idempotent.
            global.extra.insert(
                "skipDangerousModePermissionPrompt".into(),
                Value::Bool(true),
            );
            global.extra.remove("bypassPermissionsModeAccepted");
            true
        }
        _ => false,
    }
}

pub fn reset_auto_mode_opt_in_for_default_offer(
    ctx: &MigrationContext,
    global: &mut GlobalConfig,
    settings: &mut Settings,
) -> bool {
    if !ctx.transcript_classifier_enabled {
        return false;
    }
    if matches!(
        global.extra.get("hasResetAutoModeOptInForDefaultOffer"),
        Some(Value::Bool(true))
    ) {
        return false;
    }
    if ctx.auto_mode_enabled_state.as_deref() != Some("enabled") {
        return false;
    }

    if settings.skip_auto_permission_prompt == Some(true)
        && settings.permissions.default_mode.as_deref() != Some("auto")
    {
        settings.skip_auto_permission_prompt = None;
    }
    global.extra.insert(
        "hasResetAutoModeOptInForDefaultOffer".into(),
        Value::Bool(true),
    );
    true
}

pub fn migrate_project_mcp_approval_fields(
    project_config: &mut serde_json::Map<String, Value>,
    local_settings: &mut Settings,
) -> bool {
    let mut changed = false;
    if let Some(value) = project_config.remove("enableAllProjectMcpServers") {
        if local_settings.enable_all_project_mcp_servers.is_none() {
            local_settings.enable_all_project_mcp_servers = value.as_bool();
        }
        changed = true;
    }
    for (key, target) in [
        (
            "enabledMcpjsonServers",
            &mut local_settings.enabled_mcpjson_servers,
        ),
        (
            "disabledMcpjsonServers",
            &mut local_settings.disabled_mcpjson_servers,
        ),
    ] {
        if let Some(Value::Array(values)) = project_config.remove(key) {
            for value in values
                .into_iter()
                .filter_map(|value| value.as_str().map(str::to_owned))
            {
                if !target.iter().any(|existing| existing == &value) {
                    target.push(value);
                }
            }
            changed = true;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_bypass_permission() {
        let mut global = GlobalConfig::default();
        global
            .extra
            .insert("bypassPermissionsModeAccepted".into(), Value::Bool(true));
        let mut settings = Settings::default();
        assert!(migrate_bypass_permissions(&mut global, &mut settings));
        assert_eq!(
            global.extra.get("skipDangerousModePermissionPrompt"),
            Some(&Value::Bool(true))
        );
        assert!(!global.extra.contains_key("bypassPermissionsModeAccepted"));
    }

    #[test]
    fn noop_when_absent() {
        let mut global = GlobalConfig::default();
        let mut settings = Settings::default();
        assert!(!migrate_bypass_permissions(&mut global, &mut settings));
    }

    #[test]
    fn is_idempotent() {
        let mut global = GlobalConfig::default();
        global
            .extra
            .insert("bypassPermissionsModeAccepted".into(), Value::Bool(true));
        let mut settings = Settings::default();
        assert!(migrate_bypass_permissions(&mut global, &mut settings));
        assert!(!migrate_bypass_permissions(&mut global, &mut settings));
    }

    #[test]
    fn migrates_auto_updates_preference_to_settings_env() {
        let mut global = GlobalConfig::default();
        global
            .extra
            .insert("autoUpdates".into(), Value::Bool(false));
        let mut settings = Settings::default();
        assert!(migrate_auto_updates_to_settings(&mut global, &mut settings));
        assert_eq!(
            settings.env.get("DISABLE_AUTOUPDATER").map(String::as_str),
            Some("1")
        );
        assert!(!global.extra.contains_key("autoUpdates"));
    }

    #[test]
    fn auto_updates_protected_for_native_skips() {
        let mut global = GlobalConfig::default();
        global
            .extra
            .insert("autoUpdates".into(), Value::Bool(false));
        global
            .extra
            .insert("autoUpdatesProtectedForNative".into(), Value::Bool(true));
        let mut settings = Settings::default();
        assert!(!migrate_auto_updates_to_settings(
            &mut global,
            &mut settings
        ));
        assert!(settings.env.is_empty());
    }

    #[test]
    fn reset_auto_mode_clears_skip_and_marks_complete() {
        let ctx = MigrationContext {
            transcript_classifier_enabled: true,
            auto_mode_enabled_state: Some("enabled".into()),
            ..Default::default()
        };
        let mut global = GlobalConfig::default();
        let mut settings = Settings {
            skip_auto_permission_prompt: Some(true),
            ..Default::default()
        };
        assert!(reset_auto_mode_opt_in_for_default_offer(
            &ctx,
            &mut global,
            &mut settings
        ));
        assert_eq!(settings.skip_auto_permission_prompt, None);
        assert_eq!(
            global.extra.get("hasResetAutoModeOptInForDefaultOffer"),
            Some(&Value::Bool(true))
        );
    }

    #[test]
    fn migrates_project_mcp_approval_fields() {
        let mut project = serde_json::Map::new();
        project.insert("enableAllProjectMcpServers".into(), Value::Bool(true));
        project.insert(
            "enabledMcpjsonServers".into(),
            serde_json::json!(["a", "b"]),
        );
        project.insert("disabledMcpjsonServers".into(), serde_json::json!(["c"]));
        let mut local = Settings {
            enabled_mcpjson_servers: vec!["a".into()],
            ..Default::default()
        };
        assert!(migrate_project_mcp_approval_fields(
            &mut project,
            &mut local
        ));
        assert_eq!(local.enable_all_project_mcp_servers, Some(true));
        assert_eq!(local.enabled_mcpjson_servers, vec!["a", "b"]);
        assert_eq!(local.disabled_mcpjson_servers, vec!["c"]);
        assert!(project.is_empty());
    }
}
