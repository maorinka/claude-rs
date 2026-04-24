//! Port of `src/keybindings/parser.ts`.

use std::collections::BTreeMap;

/// A parsed keystroke: a key plus any active modifiers.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct ParsedKeystroke {
    pub key: String,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
    pub super_: bool,
}

/// A chord is a sequence of keystrokes (e.g. `ctrl+x ctrl+k`).
pub type Chord = Vec<ParsedKeystroke>;

/// A raw keybinding block as it comes out of JSON config.
#[derive(Debug, Clone, PartialEq)]
pub struct KeybindingBlock {
    pub context: String,
    pub bindings: BTreeMap<String, String>,
}

/// Parse a keystroke string like `"ctrl+shift+k"`. Supports modifier
/// aliases (ctrl/control, alt/opt/option, cmd/command/super/win) and
/// common special-key names (esc, return, space, arrow-arrows).
pub fn parse_keystroke(input: &str) -> ParsedKeystroke {
    let mut ks = ParsedKeystroke::default();
    for part in input.split('+') {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => ks.ctrl = true,
            "alt" | "opt" | "option" => ks.alt = true,
            "shift" => ks.shift = true,
            "meta" => ks.meta = true,
            "cmd" | "command" | "super" | "win" => ks.super_ = true,
            "esc" => ks.key = "escape".into(),
            "return" => ks.key = "enter".into(),
            "space" => ks.key = " ".into(),
            "↑" => ks.key = "up".into(),
            "↓" => ks.key = "down".into(),
            "←" => ks.key = "left".into(),
            "→" => ks.key = "right".into(),
            other => ks.key = other.to_string(),
        }
    }
    ks
}

/// Parse a chord string like `"ctrl+k ctrl+s"`. A lone space is the
/// space key binding, not a separator.
pub fn parse_chord(input: &str) -> Chord {
    if input == " " {
        return vec![parse_keystroke("space")];
    }
    input.split_whitespace().map(parse_keystroke).collect()
}

/// Parse keybinding blocks (from JSON config) into flat ParsedBindings.
pub fn parse_bindings(blocks: &[KeybindingBlock]) -> Vec<super::matching::ParsedBinding> {
    let mut out = Vec::new();
    for block in blocks {
        for (k, action) in &block.bindings {
            out.push(super::matching::ParsedBinding {
                chord: parse_chord(k),
                action: action.clone(),
                context: block.context.clone(),
            });
        }
    }
    out
}

/// Canonical string representation of a keystroke (for storage/display).
pub fn keystroke_to_string(ks: &ParsedKeystroke) -> String {
    let mut parts = Vec::new();
    if ks.ctrl {
        parts.push("ctrl");
    }
    if ks.alt {
        parts.push("alt");
    }
    if ks.shift {
        parts.push("shift");
    }
    if ks.meta {
        parts.push("meta");
    }
    if ks.super_ {
        parts.push("cmd");
    }
    let display = key_to_display_name(&ks.key);
    parts.push(&display);
    parts.join("+")
}

fn key_to_display_name(key: &str) -> String {
    match key {
        "escape" => "Esc".into(),
        " " => "Space".into(),
        "tab" => "tab".into(),
        "enter" => "Enter".into(),
        "backspace" => "Backspace".into(),
        "delete" => "Delete".into(),
        "up" => "↑".into(),
        "down" => "↓".into(),
        "left" => "←".into(),
        "right" => "→".into(),
        "pageup" => "PageUp".into(),
        "pagedown" => "PageDown".into(),
        "home" => "Home".into(),
        "end" => "End".into(),
        other => other.to_string(),
    }
}

/// Canonical string representation of a chord.
pub fn chord_to_string(chord: &Chord) -> String {
    chord
        .iter()
        .map(keystroke_to_string)
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_modifier() {
        let ks = parse_keystroke("ctrl+c");
        assert!(ks.ctrl);
        assert_eq!(ks.key, "c");
    }

    #[test]
    fn parses_multiple_modifiers() {
        let ks = parse_keystroke("ctrl+shift+p");
        assert!(ks.ctrl);
        assert!(ks.shift);
        assert_eq!(ks.key, "p");
    }

    #[test]
    fn modifier_aliases() {
        let ks = parse_keystroke("control+alt+cmd+x");
        assert!(ks.ctrl);
        assert!(ks.alt);
        assert!(ks.super_);
        assert_eq!(ks.key, "x");
    }

    #[test]
    fn special_keys() {
        assert_eq!(parse_keystroke("esc").key, "escape");
        assert_eq!(parse_keystroke("return").key, "enter");
        assert_eq!(parse_keystroke("space").key, " ");
        assert_eq!(parse_keystroke("↑").key, "up");
    }

    #[test]
    fn chord_with_multiple_keystrokes() {
        let c = parse_chord("ctrl+x ctrl+k");
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].key, "x");
        assert_eq!(c[1].key, "k");
    }

    #[test]
    fn lone_space_is_space_key() {
        let c = parse_chord(" ");
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].key, " ");
    }

    #[test]
    fn round_trip_through_string() {
        let ks = parse_keystroke("ctrl+shift+p");
        let s = keystroke_to_string(&ks);
        let ks2 = parse_keystroke(&s.to_lowercase());
        assert_eq!(ks.ctrl, ks2.ctrl);
        assert_eq!(ks.shift, ks2.shift);
        assert_eq!(ks.key, ks2.key);
    }
}
