use std::time::Duration;

use rand::Rng;

/// Policy governing how the client retries failed API requests.
pub struct RetryPolicy {
    /// Initial delay before the first retry.
    pub base_delay: Duration,
    /// Maximum number of retry attempts for general transient errors (e.g. 429, 5xx).
    pub max_retries: u32,
    /// Maximum number of retry attempts specifically for HTTP 529 (overloaded) responses.
    /// After this limit the decision switches to `FallbackToNonStreaming`.
    pub max_529_retries: u32,
    /// Maximum backoff cap used for persistent overload conditions.
    pub persistent_max_backoff: Duration,
    /// Reset cap for persistent overload backoff tracking.
    pub persistent_reset_cap: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            base_delay: Duration::from_millis(500),
            max_retries: 10,
            max_529_retries: 3,
            persistent_max_backoff: Duration::from_secs(60),
            persistent_reset_cap: Duration::from_secs(300),
        }
    }
}

/// The action the caller should take after receiving a particular HTTP status.
pub enum RetryDecision {
    /// Wait for `delay` and then retry the request.
    Retry { delay: Duration },
    /// Switch from a streaming request to a non-streaming request and try once more.
    FallbackToNonStreaming,
    /// The error is not recoverable; surface it to the caller.
    Fatal { status: u16 },
}

impl RetryPolicy {
    /// Decide what to do given an HTTP `status` code and the number of retries
    /// already attempted (`attempt` starts at 1 for the first retry).
    pub fn should_retry(&self, status: u16, attempt: u32) -> RetryDecision {
        match status {
            // 529 – API overloaded: fall back to non-streaming once the limit is reached.
            529 => {
                if attempt >= self.max_529_retries {
                    RetryDecision::FallbackToNonStreaming
                } else {
                    RetryDecision::Retry {
                        delay: self.backoff_delay(attempt),
                    }
                }
            }
            // 429 – rate-limited, 500/502/503/504 – transient server errors.
            429 | 500 | 502 | 503 | 504 => {
                if attempt >= self.max_retries {
                    RetryDecision::Fatal { status }
                } else {
                    RetryDecision::Retry {
                        delay: self.backoff_delay(attempt),
                    }
                }
            }
            // Everything else (4xx client errors, etc.) is fatal.
            _ => RetryDecision::Fatal { status },
        }
    }

    /// Compute the exponential backoff delay for the given `attempt` (1-based).
    ///
    /// Formula: `min(base_delay * 2^(attempt-1), 60s) + uniform_jitter(0..10%)`
    pub fn backoff_delay(&self, attempt: u32) -> Duration {
        const MAX_DELAY: Duration = Duration::from_secs(60);

        // Compute base * 2^(attempt-1), capped at 60 s to avoid overflow.
        let exp = attempt.saturating_sub(1);
        // Use saturating_mul via checked arithmetic to avoid overflow on large exponents.
        let base_ms = self.base_delay.as_millis() as u64;
        let multiplier: u64 = if exp < 64 { 1u64 << exp } else { u64::MAX };
        let raw_ms = base_ms.saturating_mul(multiplier);

        let capped = if raw_ms >= MAX_DELAY.as_millis() as u64 {
            MAX_DELAY
        } else {
            Duration::from_millis(raw_ms)
        };

        // Add up to 10% jitter.
        let jitter_max_ms = (capped.as_millis() as f64 * 0.10) as u64;
        let jitter_ms = if jitter_max_ms > 0 {
            rand::thread_rng().gen_range(0..=jitter_max_ms)
        } else {
            0
        };

        capped + Duration::from_millis(jitter_ms)
    }
}
