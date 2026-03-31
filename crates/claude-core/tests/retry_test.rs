use claude_core::api::retry::*;
use std::time::Duration;

#[test]
fn test_retry_policy_defaults() {
    let policy = RetryPolicy::default();
    assert_eq!(policy.base_delay, Duration::from_millis(500));
    assert_eq!(policy.max_retries, 10);
    assert_eq!(policy.max_529_retries, 3);
}

#[test]
fn test_should_retry_429() {
    let policy = RetryPolicy::default();
    let decision = policy.should_retry(429, 1);
    assert!(matches!(decision, RetryDecision::Retry { .. }));
}

#[test]
fn test_should_retry_529_within_limit() {
    let policy = RetryPolicy::default();
    let decision = policy.should_retry(529, 1);
    assert!(matches!(decision, RetryDecision::Retry { .. }));
}

#[test]
fn test_should_fallback_529_exhausted() {
    let policy = RetryPolicy::default();
    let decision = policy.should_retry(529, 3);
    assert!(matches!(decision, RetryDecision::FallbackToNonStreaming));
}

#[test]
fn test_fatal_400() {
    let policy = RetryPolicy::default();
    let decision = policy.should_retry(400, 1);
    assert!(matches!(decision, RetryDecision::Fatal { .. }));
}

#[test]
fn test_fatal_403() {
    let policy = RetryPolicy::default();
    let decision = policy.should_retry(403, 1);
    assert!(matches!(decision, RetryDecision::Fatal { .. }));
}

#[test]
fn test_backoff_exponential() {
    let policy = RetryPolicy::default();
    let d1 = policy.backoff_delay(1);
    let d2 = policy.backoff_delay(2);
    let d3 = policy.backoff_delay(3);
    assert!(d1 >= Duration::from_millis(450) && d1 <= Duration::from_millis(600));
    assert!(d2 >= Duration::from_millis(900) && d2 <= Duration::from_millis(1200));
    assert!(d3 >= Duration::from_millis(1800) && d3 <= Duration::from_millis(2400));
}

#[test]
fn test_backoff_caps_at_60s() {
    let policy = RetryPolicy::default();
    let d = policy.backoff_delay(20);
    assert!(d <= Duration::from_secs(66)); // 60s + 10% jitter
}
