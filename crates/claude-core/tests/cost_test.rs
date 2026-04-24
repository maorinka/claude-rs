use claude_core::cost::tracker::CostTracker;
use claude_core::types::usage::Usage;

fn make_usage(input: u64, output: u64, cache_read: Option<u64>, cache_write: Option<u64>) -> Usage {
    Usage {
        input_tokens: input,
        output_tokens: output,
        cache_read_input_tokens: cache_read,
        cache_creation_input_tokens: cache_write,
    }
}

#[test]
fn test_cost_tracker_new_zero_state() {
    let tracker = CostTracker::new("claude-sonnet");
    assert_eq!(tracker.total_tokens(), 0);
    assert_eq!(tracker.total_cost_usd(), 0.0);
}

#[test]
fn test_cost_tracker_add_usage_accumulates() {
    let mut tracker = CostTracker::new("claude-sonnet");
    tracker.add_usage(&make_usage(100, 50, None, None));
    tracker.add_usage(&make_usage(200, 100, None, None));
    assert_eq!(tracker.total_tokens(), 450); // 300 in + 150 out
}

#[test]
fn test_cost_tracker_sonnet_pricing() {
    let mut tracker = CostTracker::new("claude-sonnet");
    // 1M input + 1M output tokens
    tracker.add_usage(&make_usage(1_000_000, 1_000_000, None, None));
    // 3.0 + 15.0 = 18.0 USD
    let cost = tracker.total_cost_usd();
    assert!((cost - 18.0).abs() < 0.0001, "expected ~18.0, got {}", cost);
}

#[test]
fn test_cost_tracker_opus_pricing() {
    let mut tracker = CostTracker::new("claude-opus");
    tracker.add_usage(&make_usage(1_000_000, 1_000_000, None, None));
    // 15.0 + 75.0 = 90.0 USD
    let cost = tracker.total_cost_usd();
    assert!((cost - 90.0).abs() < 0.0001, "expected ~90.0, got {}", cost);
}

#[test]
fn test_cost_tracker_haiku_pricing() {
    let mut tracker = CostTracker::new("claude-haiku");
    tracker.add_usage(&make_usage(1_000_000, 1_000_000, None, None));
    // 0.25 + 1.25 = 1.5 USD
    let cost = tracker.total_cost_usd();
    assert!((cost - 1.5).abs() < 0.0001, "expected ~1.5, got {}", cost);
}

#[test]
fn test_cost_tracker_cache_tokens() {
    let mut tracker = CostTracker::new("claude-sonnet");
    tracker.add_usage(&make_usage(0, 0, Some(1_000_000), Some(1_000_000)));
    // 0.3 (cache read) + 3.75 (cache write) = 4.05 USD
    let cost = tracker.total_cost_usd();
    assert!((cost - 4.05).abs() < 0.0001, "expected ~4.05, got {}", cost);
}

#[test]
fn test_cost_tracker_summary_format() {
    let mut tracker = CostTracker::new("claude-sonnet");
    tracker.add_usage(&make_usage(100, 50, Some(10), Some(5)));
    let summary = tracker.summary();
    assert!(summary.contains("100 in"));
    assert!(summary.contains("50 out"));
    assert!(summary.contains("10 read"));
    assert!(summary.contains("5 write"));
    assert!(summary.contains("Requests: 1"));
    assert!(summary.contains("Cost: $"));
}

#[test]
fn test_cost_tracker_multiple_requests_count() {
    let mut tracker = CostTracker::new("claude-sonnet");
    for _ in 0..5 {
        tracker.add_usage(&make_usage(10, 5, None, None));
    }
    assert!(tracker.summary().contains("Requests: 5"));
    assert_eq!(tracker.total_tokens(), 75);
}
