use thiserror::Error;

#[derive(Clone, Debug, Error)]
pub enum QueryError {
    #[error("API error: {status} {message}")]
    Api { status: u16, message: String },
    #[error("Network error: {0}")]
    Network(String),
    #[error("Max output tokens exhausted after {recovery_count} retries")]
    MaxTokensExhausted { recovery_count: u32 },
    #[error("Prompt too long: {input_tokens} tokens")]
    PromptTooLong { input_tokens: u64 },
    #[error("Authentication failed: {0}")]
    Auth(String),
    #[error("Aborted by user")]
    Aborted,
    #[error("Stream timed out after {seconds}s of idle")]
    StreamTimeout { seconds: u64 },
}
