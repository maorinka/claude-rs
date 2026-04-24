//! Vim mode state machine types.
//!
//! Port of the pure-data portions of `src/vim/`:
//!   - `types.ts`   → this module
//!   - `motions.ts` classifiers (`isInclusiveMotion`, `isLinewiseMotion`)
//!
//! NOT ported: motion resolution, operators, text-objects, state
//! transitions. Those all depend on a Cursor trait + line buffer API
//! that the Rust TUI hasn't exposed yet. Porting them requires the
//! claude-tui prompt-input widget to expose cursor primitives
//! (left/right/down/up/nextVimWord/…) first.
//!
//! The shapes here are the same as TS so the TUI can build the handler
//! against them and swap in the motion implementations later.

pub mod motions;
pub mod types;

pub use motions::{is_inclusive_motion, is_linewise_motion};
pub use types::{
    create_initial_persistent_state, create_initial_vim_state, is_operator_key,
    is_text_obj_scope_key, CommandState, FindType, Operator, PersistentState, RecordedChange,
    TextObjScope, VimState, FIND_KEYS, MAX_VIM_COUNT, OPERATORS, SIMPLE_MOTIONS, TEXT_OBJ_SCOPES,
    TEXT_OBJ_TYPES,
};
