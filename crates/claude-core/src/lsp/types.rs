use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// LSP diagnostic severity levels.
/// Maps to LSP DiagnosticSeverity: 1=Error, 2=Warning, 3=Information, 4=Hint.
/// Serializes as an integer per the LSP specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
#[derive(Default)]
pub enum DiagnosticSeverity {
    Error = 1,
    Warning = 2,
    Information = 3,
    #[default]
    Hint = 4,
}

impl Serialize for DiagnosticSeverity {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u8(*self as u8)
    }
}

impl<'de> Deserialize<'de> for DiagnosticSeverity {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = u8::deserialize(deserializer)?;
        DiagnosticSeverity::from_u8(value)
            .ok_or_else(|| serde::de::Error::custom(format!("invalid severity: {}", value)))
    }
}

impl DiagnosticSeverity {
    /// Parse from the numeric LSP protocol value.
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::Error),
            2 => Some(Self::Warning),
            3 => Some(Self::Information),
            4 => Some(Self::Hint),
            _ => None,
        }
    }

    /// Display name for the severity level.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Error => "Error",
            Self::Warning => "Warning",
            Self::Information => "Information",
            Self::Hint => "Hint",
        }
    }
}

/// A position in a text document (0-based line and character).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Position {
    /// Zero-based line number.
    pub line: u32,
    /// Zero-based character offset (UTF-16 code units).
    pub character: u32,
}

impl Position {
    pub fn new(line: u32, character: u32) -> Self {
        Self { line, character }
    }
}

/// A range in a text document, defined by start and end positions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

impl Range {
    pub fn new(start: Position, end: Position) -> Self {
        Self { start, end }
    }
}

/// A location in a text document identified by URI and range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Location {
    pub uri: String,
    pub range: Range,
}

/// An LSP diagnostic (error, warning, etc.) for a text document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    /// The range at which the diagnostic applies.
    pub range: Range,

    /// The diagnostic severity (Error, Warning, Information, Hint).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<DiagnosticSeverity>,

    /// The diagnostic code (string or number from the server).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<serde_json::Value>,

    /// A human-readable string describing the source of this diagnostic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,

    /// The diagnostic message.
    pub message: String,

    /// Related diagnostic information (e.g., pointing to related symbols).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub related_information: Option<Vec<DiagnosticRelatedInformation>>,
}

/// Additional information about a diagnostic, typically pointing to a related location.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticRelatedInformation {
    pub location: Location,
    pub message: String,
}

/// A file with its diagnostics, used for batch diagnostic reporting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticFile {
    /// The file URI (file:// scheme).
    pub uri: String,
    /// Diagnostics for this file.
    pub diagnostics: Vec<Diagnostic>,
}

/// JSON-RPC 2.0 request message for the LSP protocol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

impl JsonRpcRequest {
    pub fn new(id: u64, method: &str, params: Option<serde_json::Value>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            method: method.to_string(),
            params,
        }
    }
}

/// JSON-RPC 2.0 notification (no id, no response expected).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

impl JsonRpcNotification {
    pub fn new(method: &str, params: Option<serde_json::Value>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params,
        }
    }
}

/// JSON-RPC 2.0 response message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: u64,
    #[serde(default)]
    pub result: Option<serde_json::Value>,
    #[serde(default)]
    pub error: Option<JsonRpcError>,
}

/// JSON-RPC 2.0 error object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(default)]
    pub data: Option<serde_json::Value>,
}

/// LSP protocol constants.
pub const LSP_CLIENT_NAME: &str = "claude-rs";
pub const LSP_CLIENT_VERSION: &str = "0.1.0";

/// Default timeout for LSP initialization (30 seconds).
pub const LSP_INIT_TIMEOUT_MS: u64 = 30_000;

/// Default timeout for LSP requests (60 seconds).
pub const LSP_REQUEST_TIMEOUT_MS: u64 = 60_000;

/// LSP method names used by the LSP protocol.
pub mod methods {
    pub const INITIALIZE: &str = "initialize";
    pub const INITIALIZED: &str = "initialized";
    pub const SHUTDOWN: &str = "shutdown";
    pub const EXIT: &str = "exit";
    pub const TEXT_DOCUMENT_DID_OPEN: &str = "textDocument/didOpen";
    pub const TEXT_DOCUMENT_DID_CHANGE: &str = "textDocument/didChange";
    pub const TEXT_DOCUMENT_DID_CLOSE: &str = "textDocument/didClose";
    pub const TEXT_DOCUMENT_DEFINITION: &str = "textDocument/definition";
    pub const TEXT_DOCUMENT_REFERENCES: &str = "textDocument/references";
    pub const TEXT_DOCUMENT_HOVER: &str = "textDocument/hover";
    pub const TEXT_DOCUMENT_PUBLISH_DIAGNOSTICS: &str = "textDocument/publishDiagnostics";
    pub const TEXT_DOCUMENT_DOCUMENT_SYMBOL: &str = "textDocument/documentSymbol";
}

/// Encode a JSON-RPC message with Content-Length header framing
/// as required by the LSP base protocol.
///
/// Format:
/// ```text
/// Content-Length: <length>\r\n
/// \r\n
/// <JSON content>
/// ```
pub fn encode_message(body: &str) -> Vec<u8> {
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    let mut result = Vec::with_capacity(header.len() + body.len());
    result.extend_from_slice(header.as_bytes());
    result.extend_from_slice(body.as_bytes());
    result
}

/// Parse the Content-Length header from a buffer of bytes.
/// Returns Some((content_length, header_end_position)) if a complete header is found.
/// Returns None if the buffer does not yet contain a complete header.
pub fn parse_content_length(buf: &[u8]) -> Option<(usize, usize)> {
    // Look for the \r\n\r\n separator that terminates the header section
    let header_end = buf.windows(4).position(|w| w == b"\r\n\r\n")?;

    let header_str = std::str::from_utf8(&buf[..header_end]).ok()?;

    for line in header_str.lines() {
        if let Some(value) = line.strip_prefix("Content-Length: ") {
            let length: usize = value.trim().parse().ok()?;
            return Some((length, header_end + 4));
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diagnostic_severity_from_u8() {
        assert_eq!(
            DiagnosticSeverity::from_u8(1),
            Some(DiagnosticSeverity::Error)
        );
        assert_eq!(
            DiagnosticSeverity::from_u8(2),
            Some(DiagnosticSeverity::Warning)
        );
        assert_eq!(
            DiagnosticSeverity::from_u8(3),
            Some(DiagnosticSeverity::Information)
        );
        assert_eq!(
            DiagnosticSeverity::from_u8(4),
            Some(DiagnosticSeverity::Hint)
        );
        assert_eq!(DiagnosticSeverity::from_u8(0), None);
        assert_eq!(DiagnosticSeverity::from_u8(5), None);
    }

    #[test]
    fn test_diagnostic_severity_as_str() {
        assert_eq!(DiagnosticSeverity::Error.as_str(), "Error");
        assert_eq!(DiagnosticSeverity::Warning.as_str(), "Warning");
        assert_eq!(DiagnosticSeverity::Information.as_str(), "Information");
        assert_eq!(DiagnosticSeverity::Hint.as_str(), "Hint");
    }

    #[test]
    fn test_position_serialization() {
        let pos = Position::new(10, 5);
        let json = serde_json::to_value(pos).unwrap();
        assert_eq!(json["line"], 10);
        assert_eq!(json["character"], 5);

        let deserialized: Position = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized, pos);
    }

    #[test]
    fn test_range_serialization() {
        let range = Range::new(Position::new(1, 0), Position::new(1, 10));
        let json = serde_json::to_value(range).unwrap();
        assert_eq!(json["start"]["line"], 1);
        assert_eq!(json["end"]["character"], 10);

        let deserialized: Range = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized, range);
    }

    #[test]
    fn test_diagnostic_serialization() {
        let diag = Diagnostic {
            range: Range::new(Position::new(5, 0), Position::new(5, 20)),
            severity: Some(DiagnosticSeverity::Error),
            code: Some(serde_json::json!("E0001")),
            source: Some("rustc".to_string()),
            message: "expected type `i32`, found `String`".to_string(),
            related_information: None,
        };

        let json = serde_json::to_value(&diag).unwrap();
        assert_eq!(json["message"], "expected type `i32`, found `String`");
        assert_eq!(json["severity"], 1);
        assert_eq!(json["source"], "rustc");
        assert_eq!(json["code"], "E0001");

        let deserialized: Diagnostic = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized, diag);
    }

    #[test]
    fn test_diagnostic_minimal_serialization() {
        // Diagnostic with only required fields
        let diag = Diagnostic {
            range: Range::default(),
            severity: None,
            code: None,
            source: None,
            message: "something went wrong".to_string(),
            related_information: None,
        };

        let json = serde_json::to_string(&diag).unwrap();
        // Optional fields should not appear in JSON
        assert!(!json.contains("severity"));
        assert!(!json.contains("code"));
        assert!(!json.contains("source"));
        assert!(!json.contains("related_information"));
    }

    #[test]
    fn test_json_rpc_request_serialization() {
        let req = JsonRpcRequest::new(
            1,
            "textDocument/definition",
            Some(serde_json::json!({
                "textDocument": { "uri": "file:///foo.rs" },
                "position": { "line": 10, "character": 5 }
            })),
        );

        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["jsonrpc"], "2.0");
        assert_eq!(json["id"], 1);
        assert_eq!(json["method"], "textDocument/definition");
        assert!(json["params"]["textDocument"]["uri"].is_string());
    }

    #[test]
    fn test_json_rpc_notification_no_id() {
        let notif = JsonRpcNotification::new("initialized", None);
        let json = serde_json::to_value(&notif).unwrap();
        assert_eq!(json["jsonrpc"], "2.0");
        assert_eq!(json["method"], "initialized");
        // Notifications must not have an "id" field
        assert!(json.get("id").is_none());
    }

    #[test]
    fn test_encode_message() {
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#;
        let encoded = encode_message(body);
        let expected = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
        assert_eq!(encoded, expected.as_bytes());
    }

    #[test]
    fn test_parse_content_length_valid() {
        let buf = b"Content-Length: 42\r\n\r\n{\"some\":\"json\"}";
        let result = parse_content_length(buf);
        assert_eq!(result, Some((42, 22)));
    }

    #[test]
    fn test_parse_content_length_incomplete_header() {
        let buf = b"Content-Length: 42\r\n";
        let result = parse_content_length(buf);
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_content_length_no_header() {
        let buf = b"not a valid header";
        let result = parse_content_length(buf);
        assert_eq!(result, None);
    }

    #[test]
    fn test_encode_then_parse_roundtrip() {
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"test"}"#;
        let encoded = encode_message(body);
        let (length, offset) = parse_content_length(&encoded).unwrap();
        assert_eq!(length, body.len());
        let extracted = std::str::from_utf8(&encoded[offset..offset + length]).unwrap();
        assert_eq!(extracted, body);
    }

    #[test]
    fn test_location_serialization() {
        let loc = Location {
            uri: "file:///home/user/project/src/main.rs".to_string(),
            range: Range::new(Position::new(10, 4), Position::new(10, 20)),
        };

        let json = serde_json::to_value(&loc).unwrap();
        assert_eq!(json["uri"], "file:///home/user/project/src/main.rs");

        let deserialized: Location = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized, loc);
    }

    #[test]
    fn test_json_rpc_response_with_result() {
        let json_str = r#"{"jsonrpc":"2.0","id":1,"result":{"capabilities":{}}}"#;
        let resp: JsonRpcResponse = serde_json::from_str(json_str).unwrap();
        assert_eq!(resp.id, 1);
        assert!(resp.result.is_some());
        assert!(resp.error.is_none());
    }

    #[test]
    fn test_json_rpc_response_with_error() {
        let json_str =
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"Method not found"}}"#;
        let resp: JsonRpcResponse = serde_json::from_str(json_str).unwrap();
        assert_eq!(resp.id, 1);
        assert!(resp.result.is_none());
        let err = resp.error.unwrap();
        assert_eq!(err.code, -32601);
        assert_eq!(err.message, "Method not found");
    }
}
