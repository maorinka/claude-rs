//! Reserved shortcuts. Port of `src/keybindings/reservedShortcuts.ts`.
//!
//! Tracks shortcuts that are either hardcoded in the application (cannot
//! be rebound) or intercepted by the OS/terminal before ever reaching us.
//! Consumed by the user-binding validator to surface errors/warnings.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone)]
pub struct ReservedShortcut {
    pub key: &'static str,
    pub reason: &'static str,
    pub severity: Severity,
}

/// Hardcoded shortcuts — rebinding these is an error.
pub const NON_REBINDABLE: &[ReservedShortcut] = &[
    ReservedShortcut {
        key: "ctrl+c",
        reason: "Cannot be rebound — used for interrupt/exit (hardcoded)",
        severity: Severity::Error,
    },
    ReservedShortcut {
        key: "ctrl+d",
        reason: "Cannot be rebound — used for exit (hardcoded)",
        severity: Severity::Error,
    },
    ReservedShortcut {
        key: "ctrl+m",
        reason: "Cannot be rebound — identical to Enter in terminals (both send CR)",
        severity: Severity::Error,
    },
];

/// Shortcuts intercepted by the terminal / shell / kernel.
pub const TERMINAL_RESERVED: &[ReservedShortcut] = &[
    ReservedShortcut {
        key: "ctrl+z",
        reason: "Unix process suspend (SIGTSTP)",
        severity: Severity::Warning,
    },
    ReservedShortcut {
        key: "ctrl+\\",
        reason: "Terminal quit signal (SIGQUIT)",
        severity: Severity::Error,
    },
];

/// macOS-specific shortcuts intercepted by the OS.
pub const MACOS_RESERVED: &[ReservedShortcut] = &[
    ReservedShortcut {
        key: "cmd+c",
        reason: "macOS system copy",
        severity: Severity::Error,
    },
    ReservedShortcut {
        key: "cmd+v",
        reason: "macOS system paste",
        severity: Severity::Error,
    },
    ReservedShortcut {
        key: "cmd+x",
        reason: "macOS system cut",
        severity: Severity::Error,
    },
    ReservedShortcut {
        key: "cmd+q",
        reason: "macOS quit application",
        severity: Severity::Error,
    },
    ReservedShortcut {
        key: "cmd+w",
        reason: "macOS close window/tab",
        severity: Severity::Error,
    },
    ReservedShortcut {
        key: "cmd+tab",
        reason: "macOS app switcher",
        severity: Severity::Error,
    },
    ReservedShortcut {
        key: "cmd+space",
        reason: "macOS Spotlight",
        severity: Severity::Error,
    },
];

/// Get reserved shortcuts for the current platform. Always includes the
/// non-rebindable + terminal sets; adds macOS entries on macOS.
pub fn get_reserved_shortcuts() -> Vec<ReservedShortcut> {
    let mut v: Vec<ReservedShortcut> = Vec::new();
    v.extend_from_slice(NON_REBINDABLE);
    v.extend_from_slice(TERMINAL_RESERVED);
    if cfg!(target_os = "macos") {
        v.extend_from_slice(MACOS_RESERVED);
    }
    v
}

/// Normalize a key string for comparison: lowercase, sort modifier names,
/// preserve chord separators (space-separated keystrokes). Mirrors TS
/// `normalizeKeyForComparison`.
pub fn normalize_key_for_comparison(key: &str) -> String {
    key.split_whitespace()
        .map(normalize_step)
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_step(step: &str) -> String {
    let mut modifiers: Vec<String> = Vec::new();
    let mut main_key = String::new();
    for part in step.split('+') {
        let lower = part.trim().to_ascii_lowercase();
        let canonical = match lower.as_str() {
            "ctrl" | "control" => Some("ctrl"),
            "alt" | "opt" | "option" => Some("alt"),
            "meta" => Some("meta"),
            "cmd" | "command" => Some("cmd"),
            "shift" => Some("shift"),
            _ => None,
        };
        if let Some(m) = canonical {
            modifiers.push(m.to_string());
        } else {
            main_key = lower;
        }
    }
    modifiers.sort();
    if !main_key.is_empty() {
        modifiers.push(main_key);
    }
    modifiers.join("+")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_modifier_order() {
        assert_eq!(normalize_key_for_comparison("shift+ctrl+a"), "ctrl+shift+a");
        assert_eq!(normalize_key_for_comparison("CTRL+C"), "ctrl+c");
    }

    #[test]
    fn normalizes_modifier_aliases() {
        assert_eq!(normalize_key_for_comparison("cmd+a"), "cmd+a");
        assert_eq!(normalize_key_for_comparison("command+a"), "cmd+a");
        assert_eq!(normalize_key_for_comparison("opt+a"), "alt+a");
        assert_eq!(normalize_key_for_comparison("option+a"), "alt+a");
    }

    #[test]
    fn chord_preserved() {
        assert_eq!(
            normalize_key_for_comparison("ctrl+x ctrl+k"),
            "ctrl+x ctrl+k"
        );
    }

    #[test]
    fn reserved_includes_terminal() {
        let r = get_reserved_shortcuts();
        assert!(r.iter().any(|s| s.key == "ctrl+z"));
        assert!(r.iter().any(|s| s.key == "ctrl+c"));
    }
}
