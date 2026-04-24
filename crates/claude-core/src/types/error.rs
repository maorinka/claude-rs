use thiserror::Error;

#[derive(Clone, Debug, Error)]
pub enum QueryError {
    #[error("API error: {status} {message}")]
    Api { status: u16, message: String },
    #[error("Network error: {0}")]
    Network(String),
    #[error("Max output tokens exhausted after {recovery_count} retries")]
    MaxTokensExhausted { recovery_count: u32 },
    #[error("Prompt too long")]
    PromptTooLong,
    #[error("Authentication failed: {0}")]
    Auth(String),
    #[error("Aborted by user")]
    Aborted,
    #[error("Stream timed out after {seconds}s of idle")]
    StreamTimeout { seconds: u64 },
    #[error("Stream idle timeout: no data received within the timeout window")]
    StreamIdleTimeout,
}

/// Typed error for prompt-too-long (HTTP 413 / prompt_too_long) responses.
/// Distinct from QueryError so callers can downcast it specifically.
#[derive(Debug, thiserror::Error)]
#[error("API prompt too long: {body}")]
pub struct PromptTooLongError {
    pub body: String,
}
