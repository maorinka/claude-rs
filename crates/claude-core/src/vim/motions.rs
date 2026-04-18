//! Port of the classifier helpers from `src/vim/motions.ts`.
//!
//! Motion resolution itself (`resolveMotion`, `applySingleMotion`) is NOT
//! ported — it depends on a Cursor trait the Rust TUI hasn't exposed. The
//! classifiers below are pure functions over key strings and are the
//! pieces the state-transition layer needs regardless of cursor backend.

/// Is this motion inclusive? (includes character at destination).
/// Mirrors TS `isInclusiveMotion`.
pub fn is_inclusive_motion(key: &str) -> bool {
    matches!(key, "e" | "E" | "$")
}

/// Is this motion linewise? (operates on full lines when used with
/// operators). Note: `gj`/`gk` are characterwise exclusive per `:help gj`,
/// NOT linewise. Mirrors TS `isLinewiseMotion`.
pub fn is_linewise_motion(key: &str) -> bool {
    matches!(key, "j" | "k" | "G" | "gg")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inclusive_set() {
        assert!(is_inclusive_motion("e"));
        assert!(is_inclusive_motion("E"));
        assert!(is_inclusive_motion("$"));
        assert!(!is_inclusive_motion("h"));
        assert!(!is_inclusive_motion("w"));
    }

    #[test]
    fn linewise_set() {
        assert!(is_linewise_motion("j"));
        assert!(is_linewise_motion("k"));
        assert!(is_linewise_motion("G"));
        assert!(is_linewise_motion("gg"));
        assert!(!is_linewise_motion("gj")); // characterwise per :help gj
        assert!(!is_linewise_motion("h"));
    }
}
