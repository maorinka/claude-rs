//! Keybinding validation. Port of the core logic in `src/keybindings/validate.ts`.
//!
//! Produces warnings for:
//!   - attempts to rebind non-rebindable keys (ctrl+c / ctrl+d / ctrl+m)
//!   - shortcuts known to be intercepted by the terminal / OS
//!   - duplicate bindings within the same context
//!
//! JSON duplicate-key detection is not ported (`serde_json` already dedupes
//! silently — callers who want that warning need a custom parser).

use super::parser::KeybindingBlock;
use super::reserved::{get_reserved_shortcuts, normalize_key_for_comparison, Severity};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WarningType {
    Reserved,
    NonRebindable,
    Duplicate,
    ParseError,
}

#[derive(Debug, Clone)]
pub struct KeybindingWarning {
    pub warning_type: WarningType,
    pub severity: Severity,
    pub message: String,
    pub suggestion: Option<String>,
}

/// Validate a user keybindings array (after structural parse), using the
/// list of reserved shortcuts on the current platform plus a scan for
/// duplicates within each context. Returns a flat Vec of warnings.
pub fn validate_bindings(blocks: &[KeybindingBlock]) -> Vec<KeybindingWarning> {
    let mut warnings = Vec::new();
    let reserved = get_reserved_shortcuts();

    for block in blocks {
        let mut seen: std::collections::HashMap<String, &String> = std::collections::HashMap::new();
        for (key, action) in &block.bindings {
            let normalized = normalize_key_for_comparison(key);

            // Reserved / non-rebindable checks
            for r in &reserved {
                if normalize_key_for_comparison(r.key) == normalized {
                    warnings.push(KeybindingWarning {
                        warning_type: match r.severity {
                            Severity::Error => WarningType::NonRebindable,
                            Severity::Warning => WarningType::Reserved,
                        },
                        severity: r.severity,
                        message: format!(
                            "[{}] \"{}\" -> \"{}\": {}",
                            block.context, key, action, r.reason,
                        ),
                        suggestion: Some(
                            "Pick a different shortcut — this one is intercepted before Claude Code sees it"
                                .to_string(),
                        ),
                    });
                }
            }

            // Duplicate detection within the same context
            if let Some(prev) = seen.insert(normalized.clone(), action) {
                warnings.push(KeybindingWarning {
                    warning_type: WarningType::Duplicate,
                    severity: Severity::Warning,
                    message: format!(
                        "[{}] \"{}\" bound twice (first to \"{}\", then to \"{}\")",
                        block.context, key, prev, action,
                    ),
                    suggestion: Some(
                        "Remove one of the duplicate entries or rename the conflicting shortcut"
                            .to_string(),
                    ),
                });
            }
        }
    }

    warnings
}

#[cfg(test)]
mod tests {
    use super::super::parser::KeybindingBlock;
    use super::*;
    use std::collections::BTreeMap;

    fn block(context: &str, pairs: &[(&str, &str)]) -> KeybindingBlock {
        let mut m = BTreeMap::new();
        for (k, v) in pairs {
            m.insert((*k).to_string(), (*v).to_string());
        }
        KeybindingBlock {
            context: context.to_string(),
            bindings: m,
        }
    }

    #[test]
    fn rejects_ctrl_c_rebind() {
        let blocks = vec![block("Global", &[("ctrl+c", "chat:submit")])];
        let warnings = validate_bindings(&blocks);
        assert!(warnings
            .iter()
            .any(|w| matches!(w.warning_type, WarningType::NonRebindable)));
    }

    #[test]
    fn warns_on_ctrl_z() {
        let blocks = vec![block("Chat", &[("ctrl+z", "chat:suspend")])];
        let warnings = validate_bindings(&blocks);
        assert!(warnings
            .iter()
            .any(|w| matches!(w.warning_type, WarningType::Reserved)));
    }

    #[test]
    fn detects_duplicate_within_context() {
        // Use BTreeMap + same normalized key under different case.
        let blocks = vec![block("Chat", &[("CTRL+K", "a"), ("ctrl+k", "b")])];
        let warnings = validate_bindings(&blocks);
        assert!(warnings
            .iter()
            .any(|w| matches!(w.warning_type, WarningType::Duplicate)));
    }

    #[test]
    fn ok_bindings_produce_no_warnings() {
        let blocks = vec![block("Chat", &[("ctrl+g", "chat:externalEditor")])];
        let warnings = validate_bindings(&blocks);
        assert!(warnings.is_empty());
    }
}
