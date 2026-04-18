//! One-shot settings/config migrations.
//!
//! Ports the `src/migrations/` directory from TS. The TS runner is a flat
//! sequence of stateless `migrate*()` calls invoked at CLI startup. This
//! port keeps the same shape: one function per migration, each idempotent,
//! each reading + writing the same source so re-running is a no-op.
//!
//! Not every TS migration maps cleanly: several depend on subscriber state
//! (`isProSubscriber`, `isMaxSubscriber`), feature-flag dead-code elim
//! (`bun:bundle feature('X')`), or analytics plumbing we haven't ported.
//! Those migrations are implemented to the extent the Rust config surface
//! allows and otherwise no-op; see module-level comments per migration.

pub mod model_aliases;
pub mod repl_bridge;
pub mod settings_moves;

use crate::config::global::GlobalConfig;
use crate::config::settings::Settings;

/// Context passed to each migration. Subscriber flags come from the OAuth
/// account profile when available — unset means "we don't know", which all
/// subscriber-gated migrations treat as "skip".
#[derive(Debug, Default, Clone)]
pub struct MigrationContext {
    pub is_first_party: bool,
    pub is_pro: bool,
    pub is_max: bool,
    pub is_team_premium: bool,
    /// USER_TYPE=ant check; matches TS `process.env.USER_TYPE === 'ant'`.
    pub is_ant_user: bool,
}

/// Run every migration in the canonical order. Each migration is idempotent
/// and stateless; missing config/settings features cause the migration to
/// no-op rather than error. Callers should save the mutated GlobalConfig
/// and Settings after this returns if any migration reports changes.
///
/// Returns the list of migration names that actually mutated state, useful
/// for logging at CLI startup.
pub fn run_all(
    ctx: &MigrationContext,
    global: &mut GlobalConfig,
    settings: &mut Settings,
) -> Vec<&'static str> {
    let mut applied = Vec::new();
    for (name, result) in [
        (
            "migrateReplBridgeEnabledToRemoteControlAtStartup",
            repl_bridge::migrate_repl_bridge_to_remote_control(global),
        ),
        (
            "migrateFennecToOpus",
            model_aliases::migrate_fennec_to_opus(ctx, settings),
        ),
        (
            "migrateLegacyOpusToCurrent",
            model_aliases::migrate_legacy_opus_to_current(ctx, settings),
        ),
        (
            "migrateSonnet45ToSonnet46",
            model_aliases::migrate_sonnet45_to_sonnet46(ctx, settings),
        ),
        (
            "migrateBypassPermissionsAcceptedToSettings",
            settings_moves::migrate_bypass_permissions(global, settings),
        ),
        (
            "migrateSonnet1mToSonnet45",
            model_aliases::migrate_sonnet_1m_to_sonnet_45(global, settings),
        ),
    ] {
        if result {
            applied.push(name);
        }
    }
    applied
}
