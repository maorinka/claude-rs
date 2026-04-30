//! Shared bridge permission-callback message shapes.
//!
//! Ports the pure contract from TS `src/bridge/bridgePermissionCallbacks.ts`
//! plus the REPL/remote bridge send methods that wrap those callbacks as
//! StructuredIO control events.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgePermissionResponse {
    pub behavior: BridgePermissionBehavior,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_input: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_permissions: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BridgePermissionBehavior {
    Allow,
    Deny,
}

/// TS `isBridgePermissionResponse`: the only required discriminant is
/// `behavior`, and it must be either `allow` or `deny`.
pub fn is_bridge_permission_response(value: &Value) -> bool {
    matches!(
        value.get("behavior").and_then(Value::as_str),
        Some("allow" | "deny")
    )
}

pub fn parse_bridge_permission_response(value: &Value) -> Option<BridgePermissionResponse> {
    if !is_bridge_permission_response(value) {
        return None;
    }
    serde_json::from_value(value.clone()).ok()
}

pub fn bridge_permission_request_event(
    session_id: &str,
    request_id: &str,
    tool_name: &str,
    input: Value,
    tool_use_id: &str,
    description: &str,
    permission_suggestions: Option<Value>,
    blocked_path: Option<&str>,
) -> Value {
    let mut request = json!({
        "subtype": "can_use_tool",
        "tool_name": tool_name,
        "input": input,
        "tool_use_id": tool_use_id,
        "description": description,
    });
    if let Some(suggestions) = permission_suggestions {
        request["permission_suggestions"] = suggestions;
    }
    if let Some(path) = blocked_path {
        request["blocked_path"] = json!(path);
    }
    json!({
        "type": "control_request",
        "request_id": request_id,
        "request": request,
        "session_id": session_id,
    })
}

pub fn bridge_permission_response_event(
    session_id: &str,
    request_id: &str,
    response: &BridgePermissionResponse,
) -> Value {
    json!({
        "type": "control_response",
        "response": {
            "subtype": "success",
            "request_id": request_id,
            "response": response,
        },
        "session_id": session_id,
    })
}

pub fn bridge_permission_cancel_event(session_id: &str, request_id: &str) -> Value {
    json!({
        "type": "control_cancel_request",
        "request_id": request_id,
        "session_id": session_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_bridge_permission_response_like_ts() {
        assert!(is_bridge_permission_response(&json!({"behavior": "allow"})));
        assert!(is_bridge_permission_response(&json!({"behavior": "deny"})));
        assert!(!is_bridge_permission_response(&json!({"behavior": "ask"})));
        assert!(!is_bridge_permission_response(&json!({"message": "no"})));
    }

    #[test]
    fn control_request_shape_matches_repl_bridge_callbacks() {
        let event = bridge_permission_request_event(
            "session-1",
            "req-1",
            "Bash",
            json!({"command": "pwd"}),
            "toolu-1",
            "Bash needs permission",
            Some(json!([{"behavior": "allow"}])),
            Some("/tmp/x"),
        );
        assert_eq!(event["type"], "control_request");
        assert_eq!(event["session_id"], "session-1");
        assert_eq!(event["request_id"], "req-1");
        assert_eq!(event["request"]["subtype"], "can_use_tool");
        assert_eq!(event["request"]["tool_name"], "Bash");
        assert_eq!(event["request"]["input"]["command"], "pwd");
        assert_eq!(event["request"]["tool_use_id"], "toolu-1");
        assert_eq!(event["request"]["description"], "Bash needs permission");
        assert_eq!(
            event["request"]["permission_suggestions"][0]["behavior"],
            "allow"
        );
        assert_eq!(event["request"]["blocked_path"], "/tmp/x");
    }

    #[test]
    fn control_response_and_cancel_shapes_match_ts() {
        let response = BridgePermissionResponse {
            behavior: BridgePermissionBehavior::Allow,
            updated_input: Some(json!({"command": "ls"})),
            updated_permissions: None,
            message: None,
        };
        let event = bridge_permission_response_event("session-1", "req-1", &response);
        assert_eq!(event["type"], "control_response");
        assert_eq!(event["session_id"], "session-1");
        assert_eq!(event["response"]["subtype"], "success");
        assert_eq!(event["response"]["request_id"], "req-1");
        assert_eq!(event["response"]["response"]["behavior"], "allow");
        assert_eq!(
            event["response"]["response"]["updatedInput"]["command"],
            "ls"
        );

        let cancel = bridge_permission_cancel_event("session-1", "req-1");
        assert_eq!(
            cancel,
            json!({
                "type": "control_cancel_request",
                "request_id": "req-1",
                "session_id": "session-1",
            })
        );
    }
}
