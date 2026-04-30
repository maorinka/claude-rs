//! Shared bridge message helpers.
//!
//! Ports the pure transport-layer behavior from TS `src/bridge/bridgeMessaging.ts`:
//! ingress classification, bounded UUID echo-dedup, server control-response
//! shaping, and the archival result message. Runtime bridge cores can wire these
//! helpers to whichever transport they use.

use serde_json::{json, Value};
use std::collections::{HashSet, VecDeque};

#[derive(Debug, Clone, PartialEq)]
pub enum BridgeIngress {
    User(Value),
    ControlResponse(Value),
    ControlRequest(Value),
    Ignored,
}

#[derive(Debug, Clone)]
pub struct BoundedUuidSet {
    capacity: usize,
    queue: VecDeque<String>,
    set: HashSet<String>,
}

impl BoundedUuidSet {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            queue: VecDeque::with_capacity(capacity),
            set: HashSet::with_capacity(capacity),
        }
    }

    pub fn add(&mut self, uuid: impl Into<String>) {
        if self.capacity == 0 {
            return;
        }
        let uuid = uuid.into();
        if self.set.contains(&uuid) {
            return;
        }
        if self.queue.len() == self.capacity {
            if let Some(oldest) = self.queue.pop_front() {
                self.set.remove(&oldest);
            }
        }
        self.set.insert(uuid.clone());
        self.queue.push_back(uuid);
    }

    pub fn contains(&self, uuid: &str) -> bool {
        self.set.contains(uuid)
    }

    pub fn clear(&mut self) {
        self.queue.clear();
        self.set.clear();
    }
}

pub fn is_sdk_message(value: &Value) -> bool {
    value.get("type").and_then(Value::as_str).is_some()
}

pub fn is_sdk_control_response(value: &Value) -> bool {
    value.get("type").and_then(Value::as_str) == Some("control_response")
        && value.get("response").is_some()
}

pub fn is_sdk_control_request(value: &Value) -> bool {
    value.get("type").and_then(Value::as_str) == Some("control_request")
        && value.get("request_id").is_some()
        && value.get("request").is_some()
}

pub fn handle_ingress_message(
    data: &str,
    recent_posted_uuids: &mut BoundedUuidSet,
    recent_inbound_uuids: &mut BoundedUuidSet,
) -> BridgeIngress {
    let Ok(mut parsed) = serde_json::from_str::<Value>(data) else {
        return BridgeIngress::Ignored;
    };
    crate::control_message_compat::normalize_control_message_keys(&mut parsed);

    if is_sdk_control_response(&parsed) {
        return BridgeIngress::ControlResponse(parsed);
    }
    if is_sdk_control_request(&parsed) {
        return BridgeIngress::ControlRequest(parsed);
    }
    if !is_sdk_message(&parsed) {
        return BridgeIngress::Ignored;
    }

    let uuid = parsed.get("uuid").and_then(Value::as_str);
    if uuid.is_some_and(|uuid| recent_posted_uuids.contains(uuid)) {
        return BridgeIngress::Ignored;
    }
    if uuid.is_some_and(|uuid| recent_inbound_uuids.contains(uuid)) {
        return BridgeIngress::Ignored;
    }

    if parsed.get("type").and_then(Value::as_str) == Some("user") {
        if let Some(uuid) = uuid {
            recent_inbound_uuids.add(uuid);
        }
        BridgeIngress::User(parsed)
    } else {
        BridgeIngress::Ignored
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PermissionModeVerdict {
    Ok,
    Error(String),
    Unsupported,
}

pub fn server_control_response(
    request: &Value,
    session_id: &str,
    outbound_only: bool,
    permission_mode_verdict: PermissionModeVerdict,
) -> Value {
    let request_id = request
        .get("request_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let subtype = request
        .get("request")
        .and_then(|request| request.get("subtype"))
        .and_then(Value::as_str)
        .unwrap_or_default();

    let response = if outbound_only && subtype != "initialize" {
        bridge_control_error_response(
            request_id,
            "This session is outbound-only. Enable Remote Control locally to allow inbound control.",
        )
    } else {
        match subtype {
            "initialize" => json!({
                "type": "control_response",
                "response": {
                    "subtype": "success",
                    "request_id": request_id,
                    "response": {
                        "commands": [],
                        "output_style": "normal",
                        "available_output_styles": ["normal"],
                        "models": [],
                        "account": {},
                        "pid": std::process::id(),
                    },
                },
            }),
            "set_model" | "set_max_thinking_tokens" | "interrupt" => {
                bridge_control_success_response(request_id)
            }
            "set_permission_mode" => match permission_mode_verdict {
                PermissionModeVerdict::Ok => bridge_control_success_response(request_id),
                PermissionModeVerdict::Error(error) => {
                    bridge_control_error_response(request_id, &error)
                }
                PermissionModeVerdict::Unsupported => bridge_control_error_response(
                    request_id,
                    "set_permission_mode is not supported in this context (onSetPermissionMode callback not registered)",
                ),
            },
            other => bridge_control_error_response(
                request_id,
                &format!("REPL bridge does not handle control_request subtype: {other}"),
            ),
        }
    };

    let mut event = response;
    event["session_id"] = json!(session_id);
    event
}

fn bridge_control_success_response(request_id: &str) -> Value {
    json!({
        "type": "control_response",
        "response": {
            "subtype": "success",
            "request_id": request_id,
        },
    })
}

fn bridge_control_error_response(request_id: &str, error: &str) -> Value {
    json!({
        "type": "control_response",
        "response": {
            "subtype": "error",
            "request_id": request_id,
            "error": error,
        },
    })
}

pub fn make_result_message(session_id: &str) -> Value {
    json!({
        "type": "result",
        "subtype": "success",
        "duration_ms": 0,
        "duration_api_ms": 0,
        "is_error": false,
        "num_turns": 0,
        "result": "",
        "stop_reason": null,
        "total_cost_usd": 0.0,
        "usage": {
            "input_tokens": 0,
            "cache_creation_input_tokens": 0,
            "cache_read_input_tokens": 0,
            "output_tokens": 0,
            "server_tool_use": null,
            "service_tier": null,
        },
        "modelUsage": {},
        "permission_denials": [],
        "session_id": session_id,
        "uuid": uuid::Uuid::new_v4().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_uuid_set_evicts_fifo_like_ts_ring() {
        let mut set = BoundedUuidSet::new(2);
        set.add("a");
        set.add("b");
        set.add("b");
        set.add("c");
        assert!(!set.contains("a"));
        assert!(set.contains("b"));
        assert!(set.contains("c"));
        set.clear();
        assert!(!set.contains("b"));
    }

    #[test]
    fn ingress_routes_control_messages_before_sdk_message() {
        let mut posted = BoundedUuidSet::new(10);
        let mut inbound = BoundedUuidSet::new(10);
        let routed = handle_ingress_message(
            r#"{"type":"control_response","response":{"requestId":"req-1","subtype":"success"}}"#,
            &mut posted,
            &mut inbound,
        );
        match routed {
            BridgeIngress::ControlResponse(value) => {
                assert_eq!(value["response"]["request_id"], json!("req-1"));
            }
            _ => panic!("expected control response"),
        }

        let routed = handle_ingress_message(
            r#"{"type":"control_request","requestId":"req-2","request":{"subtype":"initialize"}}"#,
            &mut posted,
            &mut inbound,
        );
        match routed {
            BridgeIngress::ControlRequest(value) => {
                assert_eq!(value["request_id"], json!("req-2"));
            }
            _ => panic!("expected control request"),
        }
    }

    #[test]
    fn ingress_dedups_echoes_and_redelivered_user_messages() {
        let mut posted = BoundedUuidSet::new(10);
        let mut inbound = BoundedUuidSet::new(10);
        posted.add("posted-1");
        assert_eq!(
            handle_ingress_message(
                r#"{"type":"user","uuid":"posted-1","message":{"content":"echo"}}"#,
                &mut posted,
                &mut inbound,
            ),
            BridgeIngress::Ignored
        );
        let first = handle_ingress_message(
            r#"{"type":"user","uuid":"in-1","message":{"content":"hi"}}"#,
            &mut posted,
            &mut inbound,
        );
        assert!(matches!(first, BridgeIngress::User(_)));
        assert_eq!(
            handle_ingress_message(
                r#"{"type":"user","uuid":"in-1","message":{"content":"hi"}}"#,
                &mut posted,
                &mut inbound,
            ),
            BridgeIngress::Ignored
        );
        assert_eq!(
            handle_ingress_message(
                r#"{"type":"assistant","uuid":"a-1","message":{"content":"ignored"}}"#,
                &mut posted,
                &mut inbound,
            ),
            BridgeIngress::Ignored
        );
    }

    #[test]
    fn server_control_responses_match_ts_shapes() {
        let initialize = server_control_response(
            &json!({
                "type": "control_request",
                "request_id": "req-init",
                "request": {"subtype": "initialize"}
            }),
            "session-1",
            false,
            PermissionModeVerdict::Unsupported,
        );
        assert_eq!(initialize["response"]["subtype"], "success");
        assert_eq!(initialize["response"]["response"]["output_style"], "normal");
        assert_eq!(initialize["session_id"], "session-1");

        let unsupported_permission = server_control_response(
            &json!({
                "type": "control_request",
                "request_id": "req-perm",
                "request": {"subtype": "set_permission_mode", "mode": "plan"}
            }),
            "session-1",
            false,
            PermissionModeVerdict::Unsupported,
        );
        assert_eq!(unsupported_permission["response"]["subtype"], "error");

        let outbound = server_control_response(
            &json!({
                "type": "control_request",
                "request_id": "req-model",
                "request": {"subtype": "set_model"}
            }),
            "session-1",
            true,
            PermissionModeVerdict::Unsupported,
        );
        assert_eq!(outbound["response"]["subtype"], "error");
        assert_eq!(
            outbound["response"]["error"],
            "This session is outbound-only. Enable Remote Control locally to allow inbound control."
        );
    }

    #[test]
    fn archival_result_message_matches_ts_contract() {
        let value = make_result_message("session-1");
        assert_eq!(value["type"], "result");
        assert_eq!(value["subtype"], "success");
        assert_eq!(value["usage"]["input_tokens"], 0);
        assert_eq!(value["modelUsage"], json!({}));
        assert_eq!(value["permission_denials"], json!([]));
        assert_eq!(value["session_id"], "session-1");
        assert!(value["uuid"].as_str().is_some());
    }
}
