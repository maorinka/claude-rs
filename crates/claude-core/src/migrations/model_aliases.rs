//! Model-alias migrations.
//!
//! Ports `migrateFennecToOpus.ts`, `migrateLegacyOpusToCurrent.ts`,
//! `migrateSonnet45ToSonnet46.ts`, and `migrateSonnet1mToSonnet45.ts`.
//! All four operate on `settings.model` string and translate legacy
//! aliases to the current canonical ones.
//!
//! Fast-mode / analytics side-effects from TS are not ported — the Rust
//! Settings struct currently has no `fast_mode` field. When callers care
//! about surfacing a one-time notification, they can read the migration's
//! return value (true means changed).

use super::MigrationContext;
use crate::config::global::GlobalConfig;
use crate::config::settings::Settings;
use serde_json::Value;

/// Port of `migrateFennecToOpus`:
///   - `fennec-latest[1m]` → `opus[1m]`
///   - `fennec-latest`     → `opus`
///   - `fennec-fast-latest`, `opus-4-5-fast` → `opus[1m]` (+ fastMode — N/A in Rust)
///
/// TS gates this on `USER_TYPE=ant`. Returns true if `settings.model` changed.
pub fn migrate_fennec_to_opus(ctx: &MigrationContext, settings: &mut Settings) -> bool {
    if !ctx.is_ant_user {
        return false;
    }
    let Some(model) = settings.model.clone() else {
        return false;
    };
    let new_model = if model.starts_with("fennec-latest[1m]") {
        Some("opus[1m]".to_string())
    } else if model.starts_with("fennec-latest") {
        Some("opus".to_string())
    } else if model.starts_with("fennec-fast-latest") || model.starts_with("opus-4-5-fast") {
        Some("opus[1m]".to_string())
    } else {
        None
    };
    match new_model {
        Some(m) if m != model => {
            settings.model = Some(m);
            true
        },
        _ => false,
    }
}

/// Port of `migrateLegacyOpusToCurrent`: 1P users with explicit
/// Opus 4.0/4.1 model strings get bumped to the `opus` alias. TS does the
/// actual remap via `parseUserSpecifiedModel` at runtime; this migration
/// normalises the stored setting so `/model` displays the right thing.
pub fn migrate_legacy_opus_to_current(ctx: &MigrationContext, settings: &mut Settings) -> bool {
    if !ctx.is_first_party {
        return false;
    }
    let Some(model) = settings.model.as_deref() else {
        return false;
    };
    let canonical = match model {
        "claude-opus-4-20250514" | "claude-opus-4-1-20250805" | "opus-4" | "opus-4-1" => "opus",
        "claude-opus-4-20250514[1m]"
        | "claude-opus-4-1-20250805[1m]"
        | "opus-4[1m]"
        | "opus-4-1[1m]" => "opus[1m]",
        _ => return false,
    };
    if settings.model.as_deref() == Some(canonical) {
        return false;
    }
    settings.model = Some(canonical.to_string());
    true
}

/// Port of `migrateSonnet45ToSonnet46`: 1P Pro/Max/TeamPremium subscribers
/// with an explicit Sonnet 4.5 model string get bumped to the `sonnet`
/// alias (which now resolves to Sonnet 4.6).
pub fn migrate_sonnet45_to_sonnet46(ctx: &MigrationContext, settings: &mut Settings) -> bool {
    if !ctx.is_first_party {
        return false;
    }
    if !(ctx.is_pro || ctx.is_max || ctx.is_team_premium) {
        return false;
    }
    let Some(model) = settings.model.clone() else {
        return false;
    };
    let canonical = match model.as_str() {
        "claude-sonnet-4-5-20250929" | "sonnet-4-5-20250929" => "sonnet",
        "claude-sonnet-4-5-20250929[1m]" | "sonnet-4-5-20250929[1m]" => "sonnet[1m]",
        _ => return false,
    };
    if model == canonical {
        return false;
    }
    settings.model = Some(canonical.to_string());
    true
}

/// Port of `migrateSonnet1mToSonnet45`: users who had `sonnet[1m]` saved
/// get pinned to the explicit `sonnet-4-5-20250929[1m]` before the
/// `sonnet` alias flips to 4.6. Runs exactly once — completion is
/// tracked via `sonnet1m45MigrationComplete` in GlobalConfig.extra.
/// Returns true if state changed.
pub fn migrate_sonnet_1m_to_sonnet_45(global: &mut GlobalConfig, settings: &mut Settings) -> bool {
    // Completion flag already set → no-op.
    if matches!(
        global.extra.get("sonnet1m45MigrationComplete"),
        Some(Value::Bool(true))
    ) {
        return false;
    }

    if settings.model.as_deref() == Some("sonnet[1m]") {
        settings.model = Some("sonnet-4-5-20250929[1m]".into());
    }

    // Mark completion regardless of whether the model matched, so we never
    // re-enter. Matches TS: saveGlobalConfig always runs at the end.
    global
        .extra
        .insert("sonnet1m45MigrationComplete".into(), Value::Bool(true));
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings_with_model(m: &str) -> Settings {
        Settings {
            model: Some(m.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn fennec_gated_on_ant() {
        let mut settings = settings_with_model("fennec-latest");
        let ctx = MigrationContext {
            is_ant_user: false,
            ..Default::default()
        };
        assert!(!migrate_fennec_to_opus(&ctx, &mut settings));
        assert_eq!(settings.model.as_deref(), Some("fennec-latest"));
    }

    #[test]
    fn fennec_1m_variant_mapped() {
        let mut settings = settings_with_model("fennec-latest[1m]");
        let ctx = MigrationContext {
            is_ant_user: true,
            ..Default::default()
        };
        assert!(migrate_fennec_to_opus(&ctx, &mut settings));
        assert_eq!(settings.model.as_deref(), Some("opus[1m]"));
    }

    #[test]
    fn fennec_fast_mapped_to_opus_1m() {
        let mut settings = settings_with_model("fennec-fast-latest");
        let ctx = MigrationContext {
            is_ant_user: true,
            ..Default::default()
        };
        assert!(migrate_fennec_to_opus(&ctx, &mut settings));
        assert_eq!(settings.model.as_deref(), Some("opus[1m]"));
    }

    #[test]
    fn legacy_opus_mapped() {
        let mut settings = settings_with_model("claude-opus-4-1-20250805");
        let ctx = MigrationContext {
            is_first_party: true,
            ..Default::default()
        };
        assert!(migrate_legacy_opus_to_current(&ctx, &mut settings));
        assert_eq!(settings.model.as_deref(), Some("opus"));
    }

    #[test]
    fn sonnet45_only_for_paid_subscriber() {
        let mut settings = settings_with_model("claude-sonnet-4-5-20250929");
        let free_ctx = MigrationContext {
            is_first_party: true,
            ..Default::default()
        };
        assert!(!migrate_sonnet45_to_sonnet46(&free_ctx, &mut settings));

        let pro_ctx = MigrationContext {
            is_first_party: true,
            is_pro: true,
            ..Default::default()
        };
        assert!(migrate_sonnet45_to_sonnet46(&pro_ctx, &mut settings));
        assert_eq!(settings.model.as_deref(), Some("sonnet"));
    }

    #[test]
    fn sonnet_1m_to_45_pins_model() {
        let mut g = GlobalConfig::default();
        let mut s = settings_with_model("sonnet[1m]");
        assert!(migrate_sonnet_1m_to_sonnet_45(&mut g, &mut s));
        assert_eq!(s.model.as_deref(), Some("sonnet-4-5-20250929[1m]"));
        assert_eq!(
            g.extra.get("sonnet1m45MigrationComplete"),
            Some(&Value::Bool(true))
        );
    }

    #[test]
    fn sonnet_1m_migration_idempotent() {
        let mut g = GlobalConfig::default();
        let mut s = settings_with_model("sonnet[1m]");
        assert!(migrate_sonnet_1m_to_sonnet_45(&mut g, &mut s));
        // Second call: completion flag already set.
        assert!(!migrate_sonnet_1m_to_sonnet_45(&mut g, &mut s));
    }

    #[test]
    fn sonnet_1m_migration_marks_completion_even_without_match() {
        let mut g = GlobalConfig::default();
        let mut s = settings_with_model("opus");
        assert!(migrate_sonnet_1m_to_sonnet_45(&mut g, &mut s));
        // Model unchanged.
        assert_eq!(s.model.as_deref(), Some("opus"));
        // Completion flag set so future invocations no-op.
        assert_eq!(
            g.extra.get("sonnet1m45MigrationComplete"),
            Some(&Value::Bool(true))
        );
    }

    #[test]
    fn sonnet45_1m_mapped() {
        let mut settings = settings_with_model("claude-sonnet-4-5-20250929[1m]");
        let ctx = MigrationContext {
            is_first_party: true,
            is_max: true,
            ..Default::default()
        };
        assert!(migrate_sonnet45_to_sonnet46(&ctx, &mut settings));
        assert_eq!(settings.model.as_deref(), Some("sonnet[1m]"));
    }
}
