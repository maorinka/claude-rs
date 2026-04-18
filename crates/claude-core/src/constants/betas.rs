//! Port of `src/constants/betas.ts`.
//!
//! Anthropic API beta header strings and the filter sets that decide
//! which ones go in the `anthropic-beta` header vs Bedrock `extraBodyParams`
//! vs Vertex countTokens.

use std::collections::HashSet;
use std::sync::OnceLock;

pub const CLAUDE_CODE_20250219: &str = "claude-code-20250219";
pub const INTERLEAVED_THINKING: &str = "interleaved-thinking-2025-05-14";
pub const CONTEXT_1M: &str = "context-1m-2025-08-07";
pub const CONTEXT_MANAGEMENT: &str = "context-management-2025-06-27";
pub const STRUCTURED_OUTPUTS: &str = "structured-outputs-2025-12-15";
pub const WEB_SEARCH: &str = "web-search-2025-03-05";

/// First-party tool-search beta (Claude API / Foundry).
pub const TOOL_SEARCH_1P: &str = "advanced-tool-use-2025-11-20";
/// Third-party tool-search beta (Vertex AI / Bedrock).
pub const TOOL_SEARCH_3P: &str = "tool-search-tool-2025-10-19";

pub const EFFORT: &str = "effort-2025-11-24";
pub const TASK_BUDGETS: &str = "task-budgets-2026-03-13";
pub const PROMPT_CACHING_SCOPE: &str = "prompt-caching-scope-2026-01-05";
pub const FAST_MODE: &str = "fast-mode-2026-02-01";
pub const REDACT_THINKING: &str = "redact-thinking-2026-02-12";
pub const TOKEN_EFFICIENT_TOOLS: &str = "token-efficient-tools-2026-03-28";
pub const SUMMARIZE_CONNECTOR_TEXT: &str = "summarize-connector-text-2026-03-13";
pub const AFK_MODE: &str = "afk-mode-2026-01-31";
pub const CLI_INTERNAL: &str = "cli-internal-2026-02-09";
pub const ADVISOR: &str = "advisor-tool-2026-03-01";

/// Betas that should go in Bedrock `extraBodyParams` and NOT as headers.
/// Matches TS `BEDROCK_EXTRA_PARAMS_HEADERS`.
pub fn bedrock_extra_params_headers() -> &'static HashSet<&'static str> {
    static CELL: OnceLock<HashSet<&'static str>> = OnceLock::new();
    CELL.get_or_init(|| [INTERLEAVED_THINKING, CONTEXT_1M, TOOL_SEARCH_3P].into_iter().collect())
}

/// Betas allowed on the Vertex `countTokens` API. Other betas cause 400s.
pub fn vertex_count_tokens_allowed_betas() -> &'static HashSet<&'static str> {
    static CELL: OnceLock<HashSet<&'static str>> = OnceLock::new();
    CELL.get_or_init(|| {
        [CLAUDE_CODE_20250219, INTERLEAVED_THINKING, CONTEXT_MANAGEMENT]
            .into_iter()
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_beta_ids() {
        // Sanity-check a few entries match what TS ships.
        assert_eq!(CLAUDE_CODE_20250219, "claude-code-20250219");
        assert_eq!(CONTEXT_1M, "context-1m-2025-08-07");
    }

    #[test]
    fn bedrock_set_includes_context_1m() {
        assert!(bedrock_extra_params_headers().contains(&CONTEXT_1M));
        assert!(!bedrock_extra_params_headers().contains(&CLAUDE_CODE_20250219));
    }

    #[test]
    fn vertex_count_tokens_set_includes_context_mgmt() {
        assert!(vertex_count_tokens_allowed_betas().contains(&CONTEXT_MANAGEMENT));
        assert!(!vertex_count_tokens_allowed_betas().contains(&TOOL_SEARCH_1P));
    }
}
