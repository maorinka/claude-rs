//! Port of `src/vim/types.ts`.

use std::collections::HashSet;
use std::sync::OnceLock;

// ── Core ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operator {
    Delete,
    Change,
    Yank,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindType {
    F,
    FUpper,
    T,
    TUpper,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextObjScope {
    Inner,
    Around,
}

// ── State machine ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandState {
    Idle,
    Count {
        digits: String,
    },
    Operator {
        op: Operator,
        count: u32,
    },
    OperatorCount {
        op: Operator,
        count: u32,
        digits: String,
    },
    OperatorFind {
        op: Operator,
        count: u32,
        find: FindType,
    },
    OperatorTextObj {
        op: Operator,
        count: u32,
        scope: TextObjScope,
    },
    Find {
        find: FindType,
        count: u32,
    },
    G {
        count: u32,
    },
    OperatorG {
        op: Operator,
        count: u32,
    },
    Replace {
        count: u32,
    },
    Indent {
        dir: IndentDir,
        count: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndentDir {
    Shift,   // '>' — shift right
    Unshift, // '<' — shift left
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VimState {
    Insert { inserted_text: String },
    Normal { command: CommandState },
}

#[derive(Debug, Clone)]
pub struct PersistentState {
    pub last_change: Option<RecordedChange>,
    pub last_find: Option<LastFind>,
    pub register: String,
    pub register_is_linewise: bool,
}

#[derive(Debug, Clone)]
pub struct LastFind {
    pub find: FindType,
    pub char: char,
}

/// Recorded change for dot-repeat — captures everything needed to replay
/// a command.
#[derive(Debug, Clone)]
pub enum RecordedChange {
    Insert {
        text: String,
    },
    Operator {
        op: Operator,
        motion: String,
        count: u32,
    },
    OperatorTextObj {
        op: Operator,
        obj_type: String,
        scope: TextObjScope,
        count: u32,
    },
    OperatorFind {
        op: Operator,
        find: FindType,
        char: char,
        count: u32,
    },
    Replace {
        char: char,
        count: u32,
    },
    X {
        count: u32,
    },
    ToggleCase {
        count: u32,
    },
    Indent {
        dir: IndentDir,
        count: u32,
    },
    OpenLine {
        direction: OpenLineDir,
    },
    Join {
        count: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenLineDir {
    Above,
    Below,
}

// ── Key groups ─────────────────────────────────────────────────────────────

/// Operator keys → operator. Matches TS `OPERATORS`.
pub const OPERATORS: &[(char, Operator)] = &[
    ('d', Operator::Delete),
    ('c', Operator::Change),
    ('y', Operator::Yank),
];

pub fn is_operator_key(key: char) -> Option<Operator> {
    OPERATORS.iter().find(|(c, _)| *c == key).map(|(_, o)| *o)
}

/// Text-object scope keys → scope.
pub const TEXT_OBJ_SCOPES: &[(char, TextObjScope)] = &[
    ('i', TextObjScope::Inner),
    ('a', TextObjScope::Around),
];

pub fn is_text_obj_scope_key(key: char) -> Option<TextObjScope> {
    TEXT_OBJ_SCOPES
        .iter()
        .find(|(c, _)| *c == key)
        .map(|(_, s)| *s)
}

/// Simple cursor motions. Lazy-initialised HashSet so
/// `is_simple_motion` is O(1).
pub fn simple_motions() -> &'static HashSet<char> {
    static CELL: OnceLock<HashSet<char>> = OnceLock::new();
    CELL.get_or_init(|| {
        [
            'h', 'l', 'j', 'k', // basic movement
            'w', 'b', 'e', 'W', 'B', 'E', // word motions
            '0', '^', '$', // line positions
        ]
        .into_iter()
        .collect()
    })
}

pub fn find_keys() -> &'static HashSet<char> {
    static CELL: OnceLock<HashSet<char>> = OnceLock::new();
    CELL.get_or_init(|| ['f', 'F', 't', 'T'].into_iter().collect())
}

pub fn text_obj_types() -> &'static HashSet<char> {
    static CELL: OnceLock<HashSet<char>> = OnceLock::new();
    CELL.get_or_init(|| {
        [
            'w', 'W', // word / WORD
            '"', '\'', '`', // quotes
            '(', ')', 'b', // parens
            '[', ']', // brackets
            '{', '}', 'B', // braces
            '<', '>', // angle brackets
        ]
        .into_iter()
        .collect()
    })
}

// The TS side exposes these as Sets; keep names that roughly match.
pub static SIMPLE_MOTIONS: fn() -> &'static HashSet<char> = simple_motions;
pub static FIND_KEYS: fn() -> &'static HashSet<char> = find_keys;
pub static TEXT_OBJ_TYPES: fn() -> &'static HashSet<char> = text_obj_types;

pub const MAX_VIM_COUNT: u32 = 10000;

// ── Factories ──────────────────────────────────────────────────────────────

pub fn create_initial_vim_state() -> VimState {
    VimState::Insert {
        inserted_text: String::new(),
    }
}

pub fn create_initial_persistent_state() -> PersistentState {
    PersistentState {
        last_change: None,
        last_find: None,
        register: String::new(),
        register_is_linewise: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operator_lookup_works() {
        assert_eq!(is_operator_key('d'), Some(Operator::Delete));
        assert_eq!(is_operator_key('c'), Some(Operator::Change));
        assert_eq!(is_operator_key('y'), Some(Operator::Yank));
        assert_eq!(is_operator_key('x'), None);
    }

    #[test]
    fn text_obj_scope_lookup_works() {
        assert_eq!(is_text_obj_scope_key('i'), Some(TextObjScope::Inner));
        assert_eq!(is_text_obj_scope_key('a'), Some(TextObjScope::Around));
        assert_eq!(is_text_obj_scope_key('z'), None);
    }

    #[test]
    fn simple_motions_include_expected() {
        let s = simple_motions();
        assert!(s.contains(&'h'));
        assert!(s.contains(&'$'));
        assert!(s.contains(&'w'));
        assert!(!s.contains(&'z'));
    }

    #[test]
    fn initial_state_is_insert() {
        let s = create_initial_vim_state();
        assert!(matches!(s, VimState::Insert { .. }));
    }

    #[test]
    fn initial_persistent_state_empty() {
        let p = create_initial_persistent_state();
        assert!(p.last_change.is_none());
        assert!(p.last_find.is_none());
        assert!(p.register.is_empty());
    }

    #[test]
    fn command_state_idle_default_shape() {
        let c = CommandState::Idle;
        assert_eq!(c, CommandState::Idle);
    }
}
