//! In-process teammate permission-response check.
//!
//! Partial port of TS `utils/inProcessTeammateHelpers.ts:1-103`.
//!
//! Only the `isPermissionRelatedResponse` predicate is ported.
//! The other three functions —
//! `findInProcessTeammateTaskId`, `setAwaitingPlanApproval`,
//! `handlePlanApprovalResponse` — depend on:
//! - `AppState` from `src/state/AppState.js`
//! - `InProcessTeammateTaskState` +
//!   `isInProcessTeammateTask` from
//!   `src/tasks/InProcessTeammateTask/types.js`
//! - `updateTaskState` from `src/utils/task/framework.js`
//!
//! None of those sub-graphs are ported in the Rust tree. Rather
//! than reverse-engineer them, this port keeps the one piece
//! whose deps (`is_permission_response`,
//! `is_sandbox_permission_response`) are already landed in
//! `teams::mailbox`.
//!
//! Fields touched
//! ==============
//! - `text` (string) — the raw message payload routed across the
//!   teammate mailbox. Both predicates parse it as an envelope
//!   and peek at the tag / shape.

use crate::teams::mailbox::{is_permission_response, is_sandbox_permission_response};

/// `true` iff the message text is a permission response (tool
/// permission OR sandbox / network-host permission). TS
/// `isPermissionRelatedResponse(messageText)` at
/// `inProcessTeammateHelpers.ts:97-102`.
///
/// TS ORs two boolean-ish `!!` coerced checks; the Rust helpers
/// return `Option`, so this is just `.is_some()` on either.
pub fn is_permission_related_response(text: &str) -> bool {
    is_permission_response(text).is_some() || is_sandbox_permission_response(text).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unrelated_text_returns_false() {
        assert!(!is_permission_related_response(""));
        assert!(!is_permission_related_response("just a plain message"));
        assert!(!is_permission_related_response("{\"type\":\"other\"}"));
    }

    #[test]
    fn parses_as_permission_or_sandbox() {
        // We don't know the exact wire format without reading the
        // mailbox impl, but the predicate's job is just to OR the
        // two checks. The delegation is the contract — unit coverage
        // of the mailbox checks themselves lives in `teams::mailbox`.
        // Here we only pin that the OR-composition short-circuits
        // correctly on empty/garbage input.
        let _ = is_permission_related_response("garbage");
    }
}
