//! Config → settings migrations.
//!
//! Ports:
//! - `migrateBypassPermissionsAcceptedToSettings.ts`
//!
//! The other config-to-settings migrations
//! (`migrateAutoUpdatesToSettings`, `migrateEnableAllProjectMcpServersToSettings`)
//! depend on fields the Rust Settings / GlobalConfig types don't yet expose
//! — porting them requires growing those structs. Tracked as future work
//! rather than stubbed here.

use crate::config::global::GlobalConfig;
use crate::config::settings::Settings;
use serde_json::Value;

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
}
