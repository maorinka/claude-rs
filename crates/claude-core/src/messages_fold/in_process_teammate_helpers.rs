//! In-process teammate helpers — task-id lookup, plan-approval flag
//! update, permission-response check.
//!
//! Port of TS `utils/inProcessTeammateHelpers.ts:1-103`.
//!
//! **Reconstructed types disclaimer.** The TS file imports:
//! - `AppState` from `src/state/AppState.js` — a `DeepImmutable`
//!   object with a `tasks: { [taskId: string]: TaskState }` field
//!   (verified at `src/state/AppStateStore.ts:160`).
//! - `InProcessTeammateTaskState` +
//!   `isInProcessTeammateTask` from
//!   `src/tasks/InProcessTeammateTask/types.ts:78` — the
//!   discriminator is literally `task.type === 'in_process_teammate'`.
//! - `updateTaskState` from `src/utils/task/framework.js` — applies
//!   an immutable update over `appState.tasks[taskId]`.
//!
//! None of those type graphs are ported in the Rust tree. Rather
//! than force-port a 569-line `AppState` + the task framework, this
//! port accepts an `AppState` as `serde_json::Value` — the same
//! on-the-wire shape the TS state serialises to for log-replay and
//! the `--resume` path — and documents exactly which fields the four
//! helpers touch.
//!
//! Fields touched
//! ==============
//! - `app_state.tasks` (object) — iterated by `Value` keys; each
//!   entry must be an object.
//! - `task.type` — discriminator, must equal `"in_process_teammate"`
//!   (from `tasks/InProcessTeammateTask/types.ts:85`).
//! - `task.identity.agentName` — compared against the supplied
//!   agent name in [`find_in_process_teammate_task_id`].
//! - `task.id` — returned by that lookup.
//! - `task.awaitingPlanApproval` (bool) — written by
//!   [`set_awaiting_plan_approval`]; TS code spread-writes via
//!   `{...task, awaitingPlanApproval: …}`.
//!
//! For the permission-response check, only `text` is touched, via
//! delegation to the already-ported `teams::mailbox` helpers.

use crate::teams::mailbox::{is_permission_response, is_sandbox_permission_response};
use serde_json::Value;

/// Discriminator check. TS
/// `tasks/InProcessTeammateTask/types.ts:78-87` `isInProcessTeammateTask`.
fn is_in_process_teammate_task(task: &Value) -> bool {
    task.get("type").and_then(Value::as_str) == Some("in_process_teammate")
}

/// Find the task-id of an in-process teammate by agent name. TS
/// `findInProcessTeammateTaskId`.
///
/// Walks `app_state.tasks` values, matches on
/// `task.type === 'in_process_teammate'` + `task.identity.agentName`,
/// returns `task.id`. Returns `None` when no match found or when
/// the shape isn't recognisable.
pub fn find_in_process_teammate_task_id(
    agent_name: &str,
    app_state: &Value,
) -> Option<String> {
    let tasks = app_state.get("tasks")?.as_object()?;
    for task in tasks.values() {
        if !is_in_process_teammate_task(task) {
            continue;
        }
        let name_on_task = task
            .get("identity")
            .and_then(|i| i.get("agentName"))
            .and_then(Value::as_str);
        if name_on_task == Some(agent_name) {
            return task.get("id").and_then(Value::as_str).map(String::from);
        }
    }
    None
}

/// Toggle the `awaitingPlanApproval` flag on a single task. TS
/// `setAwaitingPlanApproval(taskId, setAppState, awaiting)`.
///
/// The TS version wraps React's `setAppState(prev => …)` pattern.
/// Rust doesn't have the closure-based immutable-setter contract,
/// so this helper takes a `&mut Value` view of AppState and mutates
/// the task in place. Returns `true` iff the task was found and
/// updated — lets callers log "task vanished mid-response" cases
/// that TS silently no-ops on.
pub fn set_awaiting_plan_approval(
    app_state: &mut Value,
    task_id: &str,
    awaiting: bool,
) -> bool {
    let Some(tasks) = app_state.get_mut("tasks").and_then(|v| v.as_object_mut()) else {
        return false;
    };
    let Some(task) = tasks.get_mut(task_id).and_then(|v| v.as_object_mut()) else {
        return false;
    };
    task.insert("awaitingPlanApproval".into(), Value::from(awaiting));
    true
}

/// Reset `awaitingPlanApproval` to `false` in response to a plan-
/// approval message arriving. TS
/// `handlePlanApprovalResponse(taskId, response, setAppState)` at
/// `inProcessTeammateHelpers.ts:77-83`.
///
/// The `_response` parameter in TS is reserved for future use
/// ("The permissionMode from the response is handled separately by
/// the agent loop (Task #11)"). This Rust port preserves the
/// signature with a deliberately-unused `_response: &Value` so the
/// call site can be migrated without reshuffling arguments later.
pub fn handle_plan_approval_response(
    app_state: &mut Value,
    task_id: &str,
    _response: &Value,
) -> bool {
    set_awaiting_plan_approval(app_state, task_id, false)
}

/// `true` iff the message text is a permission response (tool
/// permission OR sandbox / network-host permission). TS
/// `isPermissionRelatedResponse(messageText)` at
/// `inProcessTeammateHelpers.ts:97-102`. TS ORs two `!!`-coerced
/// boolean checks; the Rust helpers return `Option`, so this is
/// `.is_some()` on either.
pub fn is_permission_related_response(text: &str) -> bool {
    is_permission_response(text).is_some() || is_sandbox_permission_response(text).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn app_state_with_tasks(tasks: Value) -> Value {
        json!({ "tasks": tasks })
    }

    fn teammate_task(id: &str, agent_name: &str) -> Value {
        json!({
            "id": id,
            "type": "in_process_teammate",
            "identity": { "agentName": agent_name },
            "awaitingPlanApproval": false,
        })
    }

    fn other_task(id: &str) -> Value {
        json!({
            "id": id,
            "type": "agent",  // not in_process_teammate
            "identity": { "agentName": "other" },
        })
    }

    #[test]
    fn find_returns_matching_task_id() {
        let state = app_state_with_tasks(json!({
            "t1": teammate_task("t1", "researcher"),
            "t2": teammate_task("t2", "coder"),
        }));
        assert_eq!(
            find_in_process_teammate_task_id("coder", &state).as_deref(),
            Some("t2"),
        );
    }

    #[test]
    fn find_skips_non_in_process_tasks() {
        let state = app_state_with_tasks(json!({
            "t1": other_task("t1"),
            "t2": teammate_task("t2", "researcher"),
        }));
        assert_eq!(
            find_in_process_teammate_task_id("researcher", &state).as_deref(),
            Some("t2"),
        );
    }

    #[test]
    fn find_returns_none_when_no_match() {
        let state = app_state_with_tasks(json!({
            "t1": teammate_task("t1", "researcher"),
        }));
        assert_eq!(find_in_process_teammate_task_id("missing", &state), None);
    }

    #[test]
    fn find_returns_none_when_tasks_missing() {
        let state = json!({});
        assert_eq!(find_in_process_teammate_task_id("x", &state), None);
    }

    #[test]
    fn find_returns_none_on_wrong_agent_name_even_if_type_matches() {
        // Prove the name + type BOTH must match.
        let state = app_state_with_tasks(json!({
            "t1": teammate_task("t1", "not-me"),
        }));
        assert_eq!(find_in_process_teammate_task_id("me", &state), None);
    }

    #[test]
    fn find_skips_malformed_task_entries() {
        let state = app_state_with_tasks(json!({
            "broken": "not an object",
            "t1": teammate_task("t1", "researcher"),
        }));
        assert_eq!(
            find_in_process_teammate_task_id("researcher", &state).as_deref(),
            Some("t1"),
        );
    }

    #[test]
    fn set_awaiting_flips_flag_to_true() {
        let mut state = app_state_with_tasks(json!({
            "t1": teammate_task("t1", "x"),
        }));
        let updated = set_awaiting_plan_approval(&mut state, "t1", true);
        assert!(updated);
        assert_eq!(state["tasks"]["t1"]["awaitingPlanApproval"].as_bool(), Some(true));
    }

    #[test]
    fn set_awaiting_returns_false_for_missing_task() {
        let mut state = app_state_with_tasks(json!({}));
        let updated = set_awaiting_plan_approval(&mut state, "missing", true);
        assert!(!updated);
    }

    #[test]
    fn set_awaiting_returns_false_when_tasks_missing() {
        let mut state = json!({});
        assert!(!set_awaiting_plan_approval(&mut state, "t1", true));
    }

    #[test]
    fn set_awaiting_preserves_other_fields() {
        // TS spread-write equivalent: everything else must survive.
        let mut state = app_state_with_tasks(json!({
            "t1": teammate_task("t1", "researcher"),
        }));
        set_awaiting_plan_approval(&mut state, "t1", true);
        let task = &state["tasks"]["t1"];
        assert_eq!(task["id"].as_str(), Some("t1"));
        assert_eq!(task["type"].as_str(), Some("in_process_teammate"));
        assert_eq!(task["identity"]["agentName"].as_str(), Some("researcher"));
    }

    #[test]
    fn handle_plan_approval_resets_to_false() {
        let mut state = app_state_with_tasks(json!({
            "t1": {
                "id": "t1",
                "type": "in_process_teammate",
                "identity": { "agentName": "x" },
                "awaitingPlanApproval": true,
            }
        }));
        let response = json!({ "type": "plan_approval_response", "permissionMode": "auto" });
        let ok = handle_plan_approval_response(&mut state, "t1", &response);
        assert!(ok);
        assert_eq!(
            state["tasks"]["t1"]["awaitingPlanApproval"].as_bool(),
            Some(false)
        );
    }

    #[test]
    fn permission_related_unrelated_false() {
        assert!(!is_permission_related_response(""));
        assert!(!is_permission_related_response("just a plain message"));
    }

    #[test]
    fn permission_related_delegation_does_not_panic() {
        // Delegation contract — unit coverage of the mailbox predicates
        // themselves lives in `teams::mailbox`.
        let _ = is_permission_related_response("garbage");
        let _ = is_permission_related_response("{\"type\":\"unrelated\"}");
    }
}
