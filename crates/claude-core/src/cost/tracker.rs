use crate::types::usage::Usage;

/// Per-model pricing (USD per million tokens)
struct ModelPricing {
    input_per_million: f64,
    output_per_million: f64,
    cache_read_per_million: f64,
    cache_write_per_million: f64,
}

fn get_pricing(model: &str) -> ModelPricing {
    if model.contains("opus") {
        ModelPricing {
            input_per_million: 15.0,
            output_per_million: 75.0,
            cache_read_per_million: 1.5,
            cache_write_per_million: 18.75,
        }
    } else if model.contains("haiku") {
        ModelPricing {
            input_per_million: 0.25,
            output_per_million: 1.25,
            cache_read_per_million: 0.025,
            cache_write_per_million: 0.3,
        }
    } else {
        // Sonnet default
        ModelPricing {
            input_per_million: 3.0,
            output_per_million: 15.0,
            cache_read_per_million: 0.3,
            cache_write_per_million: 3.75,
        }
    }
}

/// Tracks cumulative token usage and cost across a session.
///
/// Mirrors the TS `cost-tracker.ts` / `bootstrap/state.ts` counters.
/// Created once in main.rs and threaded through the TUI and slash commands.
#[derive(Clone)]
pub struct CostTracker {
    model: String,
    total_input_tokens: u64,
    total_output_tokens: u64,
    total_cache_read_tokens: u64,
    total_cache_write_tokens: u64,
    request_count: u32,
    request_in_progress: bool,
    request_input_tokens: u64,
    request_output_tokens: u64,
    request_cache_read_tokens: u64,
    request_cache_write_tokens: u64,
}

impl CostTracker {
    pub fn new(model: &str) -> Self {
        Self {
            model: model.to_string(),
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_cache_read_tokens: 0,
            total_cache_write_tokens: 0,
            request_count: 0,
            request_in_progress: false,
            request_input_tokens: 0,
            request_output_tokens: 0,
            request_cache_read_tokens: 0,
            request_cache_write_tokens: 0,
        }
    }

    /// Accumulate token counts from a UsageUpdate event.
    ///
    /// Streaming responses can emit more than one usage event per API request.
    /// Anthropic's `message_delta.usage.output_tokens` is cumulative for the
    /// current response, so while a request is active we add only the delta from
    /// the last usage snapshot. Without that, token/cost totals climb far too
    /// fast on long streamed responses.
    pub fn add_usage(&mut self, usage: &Usage) {
        if self.request_in_progress {
            let cache_read = usage.cache_read_input_tokens.unwrap_or(0);
            let cache_write = usage.cache_creation_input_tokens.unwrap_or(0);

            self.total_input_tokens += usage.input_tokens.saturating_sub(self.request_input_tokens);
            self.total_output_tokens += usage
                .output_tokens
                .saturating_sub(self.request_output_tokens);
            self.total_cache_read_tokens +=
                cache_read.saturating_sub(self.request_cache_read_tokens);
            self.total_cache_write_tokens +=
                cache_write.saturating_sub(self.request_cache_write_tokens);

            self.request_input_tokens = self.request_input_tokens.max(usage.input_tokens);
            self.request_output_tokens = self.request_output_tokens.max(usage.output_tokens);
            self.request_cache_read_tokens = self.request_cache_read_tokens.max(cache_read);
            self.request_cache_write_tokens = self.request_cache_write_tokens.max(cache_write);
        } else {
            // Backwards-compatible path for tests/headless callers that feed
            // already-final per-request usage directly.
            self.total_input_tokens += usage.input_tokens;
            self.total_output_tokens += usage.output_tokens;
            self.total_cache_read_tokens += usage.cache_read_input_tokens.unwrap_or(0);
            self.total_cache_write_tokens += usage.cache_creation_input_tokens.unwrap_or(0);
        }
    }

    /// Increment the request counter. Called once per API request from
    /// the RequestStart event handler, not from add_usage.
    pub fn increment_request_count(&mut self) {
        self.request_count += 1;
        self.request_in_progress = true;
        self.request_input_tokens = 0;
        self.request_output_tokens = 0;
        self.request_cache_read_tokens = 0;
        self.request_cache_write_tokens = 0;
    }

    pub fn total_cost_usd(&self) -> f64 {
        let pricing = get_pricing(&self.model);
        (self.total_input_tokens as f64 * pricing.input_per_million / 1_000_000.0)
            + (self.total_output_tokens as f64 * pricing.output_per_million / 1_000_000.0)
            + (self.total_cache_read_tokens as f64 * pricing.cache_read_per_million / 1_000_000.0)
            + (self.total_cache_write_tokens as f64 * pricing.cache_write_per_million / 1_000_000.0)
    }

    /// One-line summary matching the TS `formatTotalCost` style.
    pub fn summary(&self) -> String {
        format!(
            "Tokens: {} in / {} out | Cache: {} read / {} write | Requests: {} | Cost: ${:.4}",
            self.total_input_tokens,
            self.total_output_tokens,
            self.total_cache_read_tokens,
            self.total_cache_write_tokens,
            self.request_count,
            self.total_cost_usd()
        )
    }

    /// Multi-line cost report for `/cost` slash command (mirrors TS `formatTotalCost()`).
    pub fn detailed_summary(&self) -> String {
        let cost = self.total_cost_usd();
        let cost_display = if cost > 0.5 {
            format!("${:.2}", cost)
        } else {
            format!("${:.4}", cost)
        };
        format!(
            "Total cost:            {cost}\n\
             Total input tokens:    {input}\n\
             Total output tokens:   {output}\n\
             Cache read tokens:     {cache_read}\n\
             Cache write tokens:    {cache_write}\n\
             API requests:          {reqs}\n\
             Model:                 {model}",
            cost = cost_display,
            input = self.total_input_tokens,
            output = self.total_output_tokens,
            cache_read = self.total_cache_read_tokens,
            cache_write = self.total_cache_write_tokens,
            reqs = self.request_count,
            model = self.model,
        )
    }

    /// Short header string for the TUI status bar: "42.1k tokens | $0.0312"
    pub fn header_display(&self) -> String {
        let total = self.total_tokens();
        let tokens_str = if total >= 1_000_000 {
            format!("{:.1}M", total as f64 / 1_000_000.0)
        } else if total >= 1_000 {
            format!("{:.1}k", total as f64 / 1_000.0)
        } else {
            format!("{}", total)
        };
        let cost = self.total_cost_usd();
        if cost > 0.0 {
            if cost > 0.5 {
                format!("{} tokens | ${:.2}", tokens_str, cost)
            } else {
                format!("{} tokens | ${:.4}", tokens_str, cost)
            }
        } else {
            format!("{} tokens", tokens_str)
        }
    }

    pub fn total_tokens(&self) -> u64 {
        self.total_input_tokens + self.total_output_tokens
    }
    pub fn total_input_tokens(&self) -> u64 {
        self.total_input_tokens
    }
    pub fn total_output_tokens(&self) -> u64 {
        self.total_output_tokens
    }
    pub fn request_count(&self) -> u32 {
        self.request_count
    }
    pub fn model(&self) -> &str {
        &self.model
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_usage(
        input: u64,
        output: u64,
        cache_read: Option<u64>,
        cache_write: Option<u64>,
    ) -> Usage {
        Usage {
            input_tokens: input,
            output_tokens: output,
            cache_read_input_tokens: cache_read,
            cache_creation_input_tokens: cache_write,
        }
    }

    #[test]
    fn test_add_usage_accumulates() {
        let mut tracker = CostTracker::new("claude-sonnet-4-6");
        tracker.add_usage(&make_usage(100, 50, Some(20), Some(10)));
        tracker.add_usage(&make_usage(200, 75, None, None));

        assert_eq!(tracker.total_input_tokens(), 300);
        assert_eq!(tracker.total_output_tokens(), 125);
        assert_eq!(tracker.total_tokens(), 425);
        // add_usage does NOT bump request_count — that comes from
        // increment_request_count() on RequestStart events.
        assert_eq!(tracker.request_count(), 0);
    }

    #[test]
    fn test_request_count_increments_separately() {
        let mut tracker = CostTracker::new("claude-sonnet-4-6");
        // Simulate one API request: MessageStart + MessageDelta = 2 add_usage calls
        tracker.increment_request_count();
        tracker.add_usage(&make_usage(1000, 0, Some(800), None));
        tracker.add_usage(&make_usage(0, 200, None, None));

        assert_eq!(tracker.request_count(), 1);
        assert_eq!(tracker.total_input_tokens(), 1000);
        assert_eq!(tracker.total_output_tokens(), 200);
    }

    #[test]
    fn test_streaming_usage_snapshots_do_not_overcount_output() {
        let mut tracker = CostTracker::new("claude-sonnet-4-6");
        tracker.increment_request_count();
        tracker.add_usage(&make_usage(1_000, 0, Some(800), Some(25)));
        tracker.add_usage(&make_usage(0, 10, None, None));
        tracker.add_usage(&make_usage(0, 25, None, None));
        tracker.add_usage(&make_usage(0, 40, None, None));

        assert_eq!(tracker.total_input_tokens(), 1_000);
        assert_eq!(tracker.total_output_tokens(), 40);
        assert_eq!(tracker.total_cache_read_tokens, 800);
        assert_eq!(tracker.total_cache_write_tokens, 25);
        assert_eq!(tracker.request_count(), 1);
    }

    #[test]
    fn test_new_request_resets_streaming_usage_snapshot() {
        let mut tracker = CostTracker::new("claude-sonnet-4-6");
        tracker.increment_request_count();
        tracker.add_usage(&make_usage(1_000, 0, None, None));
        tracker.add_usage(&make_usage(0, 50, None, None));

        tracker.increment_request_count();
        tracker.add_usage(&make_usage(1_200, 0, None, None));
        tracker.add_usage(&make_usage(0, 10, None, None));

        assert_eq!(tracker.total_input_tokens(), 2_200);
        assert_eq!(tracker.total_output_tokens(), 60);
        assert_eq!(tracker.request_count(), 2);
    }

    #[test]
    fn test_cost_calculation_sonnet() {
        let mut tracker = CostTracker::new("claude-sonnet-4-6");
        // 1M input tokens at $3/M = $3, 1M output at $15/M = $15
        tracker.add_usage(&make_usage(1_000_000, 1_000_000, None, None));
        let cost = tracker.total_cost_usd();
        assert!(
            (cost - 18.0).abs() < 0.01,
            "sonnet cost should be ~$18, got {}",
            cost
        );
    }

    #[test]
    fn test_cost_calculation_opus() {
        let mut tracker = CostTracker::new("claude-opus-4-6");
        tracker.add_usage(&make_usage(1_000_000, 1_000_000, None, None));
        let cost = tracker.total_cost_usd();
        assert!(
            (cost - 90.0).abs() < 0.01,
            "opus cost should be ~$90, got {}",
            cost
        );
    }

    #[test]
    fn test_header_display_formatting() {
        let mut tracker = CostTracker::new("claude-sonnet-4-6");
        assert_eq!(tracker.header_display(), "0 tokens");

        tracker.add_usage(&make_usage(400, 400, None, None));
        // 800 tokens total, below 1k threshold
        assert!(
            tracker.header_display().contains("800 tokens"),
            "got: {}",
            tracker.header_display()
        );

        tracker.add_usage(&make_usage(50_000, 50_000, None, None));
        // Now > 1k, should show "k tokens"
        assert!(
            tracker.header_display().contains("k tokens"),
            "got: {}",
            tracker.header_display()
        );
    }

    #[test]
    fn test_detailed_summary_contains_model() {
        let tracker = CostTracker::new("claude-sonnet-4-6");
        let summary = tracker.detailed_summary();
        assert!(summary.contains("claude-sonnet-4-6"));
        assert!(summary.contains("Total cost:"));
    }

    #[test]
    fn test_summary_format() {
        let mut tracker = CostTracker::new("claude-sonnet-4-6");
        tracker.increment_request_count();
        tracker.add_usage(&make_usage(100, 50, Some(20), Some(10)));
        let s = tracker.summary();
        assert!(s.contains("100 in"));
        assert!(s.contains("50 out"));
        assert!(s.contains("20 read"));
        assert!(s.contains("10 write"));
        assert!(s.contains("Requests: 1"));
    }
}
