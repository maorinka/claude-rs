//! Port of `src/keybindings/match.ts` (matching logic only).

use super::parser::{Chord, ParsedKeystroke};

/// A parsed keybinding entry: the chord that triggers it, the action
/// string to dispatch, and the context it applies in.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedBinding {
    pub chord: Chord,
    pub action: String,
    pub context: String,
}

/// Does the given ParsedKeystroke match this binding's first step?
/// For multi-keystroke chords, call repeatedly — the caller is
/// responsible for tracking the chord progress buffer.
pub fn matches(event: &ParsedKeystroke, binding: &ParsedKeystroke) -> bool {
    event.key == binding.key
        && event.ctrl == binding.ctrl
        && event.alt == binding.alt
        && event.shift == binding.shift
        && event.meta == binding.meta
        && event.super_ == binding.super_
}

#[cfg(test)]
mod tests {
    use super::super::parser::parse_keystroke;
    use super::*;

    #[test]
    fn exact_match() {
        let a = parse_keystroke("ctrl+c");
        let b = parse_keystroke("ctrl+c");
        assert!(matches(&a, &b));
    }

    #[test]
    fn different_key_does_not_match() {
        let a = parse_keystroke("ctrl+c");
        let b = parse_keystroke("ctrl+d");
        assert!(!matches(&a, &b));
    }

    #[test]
    fn different_modifier_does_not_match() {
        let a = parse_keystroke("ctrl+c");
        let b = parse_keystroke("c");
        assert!(!matches(&a, &b));
    }
}
