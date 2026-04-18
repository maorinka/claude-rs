//! Camel-case → snake-case shim for incoming control messages.
//!
//! Port of TS `src/utils/controlMessageCompat.ts`. Older iOS app
//! builds emit `requestId` because of a missing Swift CodingKeys
//! mapping; the bridge's `is_sdk_control_request` check insists on
//! `request_id`. Without this shim both the outer message and the
//! nested `response` payload's `requestId` get silently dropped.
//!
//! Behaviour (matches TS):
//! - Mutates `value` in place.
//! - Outer: if `requestId` exists and `request_id` does not, rename.
//! - Nested `response`: same rename if `response` is an object.
//! - When both `request_id` and `requestId` are present, `request_id`
//!   wins — the TS check is `'requestId' in record && !('request_id'
//!   in record)`, so we leave `request_id` untouched.

use serde_json::{Map, Value};

/// Rename `requestId` → `request_id` on `value` and any nested
/// `response` object. In-place mutation; returns the same reference
/// the caller passed.
pub fn normalize_control_message_keys(value: &mut Value) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    rename_request_id(obj);
    if let Some(response) = obj.get_mut("response").and_then(Value::as_object_mut) {
        rename_request_id(response);
    }
}

fn rename_request_id(obj: &mut Map<String, Value>) {
    if !obj.contains_key("requestId") {
        return;
    }
    if obj.contains_key("request_id") {
        // Snake-case already present → leave camelCase alone.
        return;
    }
    if let Some(v) = obj.remove("requestId") {
        obj.insert("request_id".to_string(), v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn renames_top_level_request_id() {
        let mut v = json!({ "requestId": "abc-123", "type": "control_request" });
        normalize_control_message_keys(&mut v);
        assert_eq!(v["request_id"], json!("abc-123"));
        assert!(v.get("requestId").is_none());
    }

    #[test]
    fn renames_nested_response_request_id() {
        let mut v = json!({
            "type": "control_response",
            "response": { "requestId": "xyz-1", "ok": true }
        });
        normalize_control_message_keys(&mut v);
        assert_eq!(v["response"]["request_id"], json!("xyz-1"));
        assert!(v["response"].get("requestId").is_none());
    }

    #[test]
    fn snake_case_wins_when_both_present() {
        let mut v =
            json!({ "request_id": "snake", "requestId": "camel" });
        normalize_control_message_keys(&mut v);
        assert_eq!(v["request_id"], json!("snake"));
        assert_eq!(v["requestId"], json!("camel"));
    }

    #[test]
    fn no_request_id_is_noop() {
        let mut v = json!({ "type": "ping" });
        let before = v.clone();
        normalize_control_message_keys(&mut v);
        assert_eq!(v, before);
    }

    #[test]
    fn non_object_value_is_noop() {
        let mut s = json!("plain string");
        normalize_control_message_keys(&mut s);
        assert_eq!(s, json!("plain string"));
        let mut n = json!(42);
        normalize_control_message_keys(&mut n);
        assert_eq!(n, json!(42));
        let mut arr = json!([1, 2, 3]);
        normalize_control_message_keys(&mut arr);
        assert_eq!(arr, json!([1, 2, 3]));
    }

    #[test]
    fn response_that_is_not_object_is_untouched() {
        let mut v =
            json!({ "requestId": "a", "response": "not-an-object" });
        normalize_control_message_keys(&mut v);
        assert_eq!(v["request_id"], json!("a"));
        assert_eq!(v["response"], json!("not-an-object"));
    }

    #[test]
    fn preserves_unrelated_fields() {
        let mut v = json!({
            "requestId": "1",
            "type": "control_request",
            "payload": { "a": 1 }
        });
        normalize_control_message_keys(&mut v);
        assert_eq!(v["type"], json!("control_request"));
        assert_eq!(v["payload"], json!({"a": 1}));
    }
}
