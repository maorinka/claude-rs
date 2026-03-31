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
        ModelPricing { input_per_million: 15.0, output_per_million: 75.0, cache_read_per_million: 1.5, cache_write_per_million: 18.75 }
    } else if model.contains("haiku") {
        ModelPricing { input_per_million: 0.25, output_per_million: 1.25, cache_read_per_million: 0.025, cache_write_per_million: 0.3 }
    } else {
        // Sonnet default
        ModelPricing { input_per_million: 3.0, output_per_million: 15.0, cache_read_per_million: 0.3, cache_write_per_million: 3.75 }
    }
}

pub struct CostTracker {
    model: String,
    total_input_tokens: u64,
    total_output_tokens: u64,
    total_cache_read_tokens: u64,
    total_cache_write_tokens: u64,
    request_count: u32,
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
        }
    }

    pub fn add_usage(&mut self, usage: &Usage) {
        self.total_input_tokens += usage.input_tokens;
        self.total_output_tokens += usage.output_tokens;
        self.total_cache_read_tokens += usage.cache_read_input_tokens.unwrap_or(0);
        self.total_cache_write_tokens += usage.cache_creation_input_tokens.unwrap_or(0);
        self.request_count += 1;
    }

    pub fn total_cost_usd(&self) -> f64 {
        let pricing = get_pricing(&self.model);
        (self.total_input_tokens as f64 * pricing.input_per_million / 1_000_000.0)
            + (self.total_output_tokens as f64 * pricing.output_per_million / 1_000_000.0)
            + (self.total_cache_read_tokens as f64 * pricing.cache_read_per_million / 1_000_000.0)
            + (self.total_cache_write_tokens as f64 * pricing.cache_write_per_million / 1_000_000.0)
    }

    pub fn summary(&self) -> String {
        format!(
            "Tokens: {} in / {} out | Cache: {} read / {} write | Requests: {} | Cost: ${:.4}",
            self.total_input_tokens, self.total_output_tokens,
            self.total_cache_read_tokens, self.total_cache_write_tokens,
            self.request_count, self.total_cost_usd()
        )
    }

    pub fn total_tokens(&self) -> u64 { self.total_input_tokens + self.total_output_tokens }
}
