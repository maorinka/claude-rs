use crate::types::message::StopReason;
use crate::types::error::QueryError;

#[derive(Clone, Debug)]
pub enum QueryState {
    /// Building request and calling API
    Querying,
    /// Streaming response from API
    Streaming,
    /// Executing tools after stream completes or during streaming
    ExecutingTools,
    /// Compacting history (token budget exceeded)
    Compacting,
    /// Recovering from max_output_tokens
    RecoveringMaxTokens {
        recovery_count: u32,
        escalated: bool,
    },
    /// Query complete
    Terminal {
        stop_reason: StopReason,
        transition: TransitionReason,
    },
}

#[derive(Clone, Debug)]
pub enum TransitionReason {
    Completed,
    MaxTurns,
    MaxOutputTokensEscalate,
    MaxOutputTokensRecovery,
    Aborted,
    Error(QueryError),
}
