use serde::{Deserialize, Serialize};

/// A JSON-RPC-style request sent over the bridge TCP connection.
///
/// Modeled after the TS `SDKControlRequest`: each request carries a unique
/// `id` so the caller can correlate responses, a `method` name, and
/// free-form `params`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BridgeRequest {
    /// Caller-generated unique identifier for request/response correlation.
    pub id: String,

    /// The RPC method name (e.g. `"ping"`, `"status"`, `"prompt"`).
    pub method: String,

    /// Method-specific parameters.
    #[serde(default)]
    pub params: serde_json::Value,
}

/// A JSON-RPC-style response returned over the bridge TCP connection.
///
/// Follows the same pattern as the TS `SDKControlResponse`: exactly one of
/// `result` or `error` is set.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BridgeResponse {
    /// Matches the `id` of the originating request.
    pub id: String,

    /// Present on success.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,

    /// Present on failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<BridgeError>,
}

/// Error payload inside a `BridgeResponse`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BridgeError {
    /// Numeric error code (negative values for protocol-level errors).
    pub code: i32,

    /// Human-readable description.
    pub message: String,
}

impl BridgeResponse {
    /// Convenience constructor for a successful response.
    pub fn success(id: String, result: serde_json::Value) -> Self {
        Self {
            id,
            result: Some(result),
            error: None,
        }
    }

    /// Convenience constructor for an error response.
    pub fn error(id: String, code: i32, message: String) -> Self {
        Self {
            id,
            result: None,
            error: Some(BridgeError { code, message }),
        }
    }
}
