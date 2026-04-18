//! Binding-lookup helpers. Port of the React-free half of
//! `src/keybindings/resolver.ts` + `shortcutFormat.ts`.
//!
//! Ink-dependent pieces (`resolveKey`, `resolveKeyWithChordState` with
//! full chord state + meta/escape quirk) are not ported — they need the
//! ratatui key-event shape. These lookup helpers are pure and usable
//! today from status-line / help-text rendering code.

use super::parser::chord_to_string;
use super::matching::ParsedBinding;

/// Return the display text for the configured binding of `action` in
/// `context`, or None if no binding is registered. Mirrors TS
/// `getBindingDisplayText`: last-wins so user overrides beat defaults.
pub fn get_binding_display_text(
    action: &str,
    context: &str,
    bindings: &[ParsedBinding],
) -> Option<String> {
    bindings
        .iter()
        .rev()
        .find(|b| b.action == action && b.context == context)
        .map(|b| chord_to_string(&b.chord))
}

/// Return the display text for a shortcut, falling back to `fallback`
/// when no binding is configured. Mirrors TS `getShortcutDisplay` —
/// minus the fallback analytics event (not yet wired in Rust).
pub fn get_shortcut_display(
    action: &str,
    context: &str,
    bindings: &[ParsedBinding],
    fallback: &str,
) -> String {
    get_binding_display_text(action, context, bindings).unwrap_or_else(|| fallback.to_string())
}

#[cfg(test)]
mod tests {
    use super::super::defaults::default_bindings;
    use super::super::parser::parse_bindings;
    use super::*;

    #[test]
    fn returns_binding_from_default_set() {
        let bindings = parse_bindings(&default_bindings());
        let s = get_binding_display_text("app:interrupt", "Global", &bindings);
        assert_eq!(s.as_deref(), Some("ctrl+c"));
    }

    #[test]
    fn unknown_action_returns_none() {
        let bindings = parse_bindings(&default_bindings());
        assert!(get_binding_display_text("not:a:real:action", "Global", &bindings).is_none());
    }

    #[test]
    fn wrong_context_returns_none() {
        let bindings = parse_bindings(&default_bindings());
        // app:interrupt lives in Global; Chat should find nothing.
        assert!(get_binding_display_text("app:interrupt", "Chat", &bindings).is_none());
    }

    #[test]
    fn user_override_wins_via_last_wins() {
        let mut bindings = parse_bindings(&default_bindings());
        // Append an override later in the vec so findLast picks it.
        bindings.push(ParsedBinding {
            chord: super::super::parser::parse_chord("ctrl+shift+c"),
            action: "app:interrupt".into(),
            context: "Global".into(),
        });
        let s = get_binding_display_text("app:interrupt", "Global", &bindings);
        assert_eq!(s.as_deref(), Some("ctrl+shift+c"));
    }

    #[test]
    fn shortcut_display_falls_back() {
        let bindings = parse_bindings(&default_bindings());
        let s = get_shortcut_display("fake:action", "Global", &bindings, "ctrl+x");
        assert_eq!(s, "ctrl+x");
    }

    #[test]
    fn shortcut_display_prefers_configured() {
        let bindings = parse_bindings(&default_bindings());
        let s = get_shortcut_display("app:interrupt", "Global", &bindings, "should-not-appear");
        assert_eq!(s, "ctrl+c");
    }
}
