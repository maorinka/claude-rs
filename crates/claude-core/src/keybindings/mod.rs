//! Keybindings subsystem.
//!
//! Port of the logic-only half of `src/keybindings/` from TS. Covers:
//!   - `parser.ts`          → `parser::{parse_keystroke, parse_chord,
//!                             keystroke_to_string, chord_to_string}`
//!   - `defaultBindings.ts` → `defaults::default_bindings()`
//!   - `match.ts`           → `matching::matches`
//!   - `schema.ts`          → types here (KeybindingBlock, Chord, etc.)
//!
//! NOT ported (TS/React-specific): KeybindingContext, KeybindingProviderSetup,
//! useKeybinding, useShortcutDisplay, template, validate (schema/zod-based),
//! shortcutFormat (purely presentational). The logic is here; rendering is
//! a UI concern that ratatui call sites implement directly.

pub mod defaults;
pub mod matching;
pub mod parser;

pub use defaults::default_bindings;
pub use matching::{matches, ParsedBinding};
pub use parser::{
    chord_to_string, keystroke_to_string, parse_bindings, parse_chord, parse_keystroke,
    Chord, KeybindingBlock, ParsedKeystroke,
};
