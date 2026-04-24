//! Dependency-free enum-like constant arrays referenced from settings validation.
//!
//! Port of TS `utils/configConstants.ts:1-21`.
//!
//! TS keeps these in their own file to avoid a circular import with
//! `settings.ts`; Rust doesn't have the same cycle risk, but keeping
//! them together mirrors the TS source-of-truth placement.

/// Notification-channel labels the settings schema accepts. Drives
/// `Notification.mp3` / iTerm2 escape-code / kitty OSC routing in the
/// TUI.
pub const NOTIFICATION_CHANNELS: &[&str] = &[
    "auto",
    "iterm2",
    "iterm2_with_bell",
    "terminal_bell",
    "kitty",
    "ghostty",
    "notifications_disabled",
];

/// Valid editor modes. TS comment at `configConstants.ts:14` calls out
/// that the deprecated `emacs` value is auto-migrated to `normal`
/// elsewhere, so it is NOT in this list.
pub const EDITOR_MODES: &[&str] = &["normal", "vim"];

/// Valid teammate-mode selectors.
/// - `auto` — pick based on context (default)
/// - `tmux` — traditional tmux-based teammates
/// - `in-process` — teammates running in the same process
pub const TEAMMATE_MODES: &[&str] = &["auto", "tmux", "in-process"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_channels_pin_ts_order() {
        // Consumers validate settings values against this slice; order
        // matters only for display in `/config`, but pinning catches
        // accidental drops.
        assert_eq!(
            NOTIFICATION_CHANNELS,
            &[
                "auto",
                "iterm2",
                "iterm2_with_bell",
                "terminal_bell",
                "kitty",
                "ghostty",
                "notifications_disabled",
            ]
        );
    }

    #[test]
    fn editor_modes_excludes_deprecated_emacs() {
        // Regression pin: TS comment says `emacs` is migrated to
        // `normal`, so it must NOT be in the accepted list.
        assert!(!EDITOR_MODES.contains(&"emacs"));
        assert_eq!(EDITOR_MODES, &["normal", "vim"]);
    }

    #[test]
    fn teammate_modes_pin() {
        assert_eq!(TEAMMATE_MODES, &["auto", "tmux", "in-process"]);
    }
}
