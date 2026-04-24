use serde_json::{json, Value};

/// Verbatim port of TS WebSearchTool/prompt.ts `getWebSearchPrompt()`.
/// The TS side splices this into the system prompt as behavioural
/// guidance (mandatory "Sources:" section, current-year search
/// queries, US-only note). Consumed by the system-prompt builder,
/// not by the tool registry — web_search is server-side.
///
/// TS interpolates `${currentMonthYear}` computed at call time; the
/// literal month shifts daily, so the Rust port keeps the "current
/// year" guidance generic — callers that want month-year
/// specificity should format it in at splice time.
pub const WEB_SEARCH_PROMPT: &str = include_str!("prompts/web_search.md");

/// WebSearchTool is a **server-side** tool.
///
/// Unlike regular client-side tools, web search is handled by Anthropic's API
/// server. The tool definition (`web_search_20250305`) is injected into the
/// request body by `build_request_body()` in `claude-core`, and the API handles
/// search execution internally via `server_tool_use` / `web_search_tool_result`
/// content blocks.
///
/// This struct is **NOT** registered in the tool registry because it is not a
/// client-side tool. It only provides the server tool definition that should be
/// included in the API request's `tools` array.
///
/// The TS implementation sends `web_search_20250305` in the request tools and
/// the API handles the search execution without any client-side tool_result
/// round-trip. This Rust implementation matches that behavior exactly.
pub struct WebSearchTool;

impl WebSearchTool {
    /// Returns the server tool definition that should be included in the API
    /// request's `tools` array. This matches the TS `makeToolSchema()`.
    pub fn server_tool_definition() -> Value {
        json!({
            "type": "web_search_20250305",
            "name": "web_search",
            "max_uses": 8
        })
    }

    /// Returns a server tool definition with domain restrictions.
    pub fn server_tool_definition_with_domains(
        allowed_domains: Option<&[String]>,
        blocked_domains: Option<&[String]>,
    ) -> Value {
        let mut def = json!({
            "type": "web_search_20250305",
            "name": "web_search",
            "max_uses": 8
        });

        if let Some(allowed) = allowed_domains {
            if !allowed.is_empty() {
                def["allowed_domains"] = json!(allowed);
            }
        }
        if let Some(blocked) = blocked_domains {
            if !blocked.is_empty() {
                def["blocked_domains"] = json!(blocked);
            }
        }

        def
    }

    /// Check if web search should be enabled for the current API provider.
    /// In the TS source, this checks for firstParty, Vertex (claude-4.0+ models),
    /// and Foundry providers. We default to true for first-party API usage.
    pub fn is_supported() -> bool {
        // In the Rust implementation, web search is available when using the
        // Anthropic API directly (the default provider). Provider detection
        // can be expanded when Vertex/Foundry support is added.
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_tool_definition() {
        let def = WebSearchTool::server_tool_definition();
        assert_eq!(def["type"], "web_search_20250305");
        assert_eq!(def["name"], "web_search");
        assert_eq!(def["max_uses"], 8);
    }

    #[test]
    fn test_server_tool_definition_with_allowed_domains() {
        let allowed = vec!["example.com".to_string(), "docs.rs".to_string()];
        let def = WebSearchTool::server_tool_definition_with_domains(Some(&allowed), None);
        assert_eq!(def["type"], "web_search_20250305");
        assert_eq!(def["allowed_domains"][0], "example.com");
        assert_eq!(def["allowed_domains"][1], "docs.rs");
        assert!(def.get("blocked_domains").is_none());
    }

    #[test]
    fn test_server_tool_definition_with_blocked_domains() {
        let blocked = vec!["spam.com".to_string()];
        let def = WebSearchTool::server_tool_definition_with_domains(None, Some(&blocked));
        assert_eq!(def["blocked_domains"][0], "spam.com");
        assert!(def.get("allowed_domains").is_none());
    }

    #[test]
    fn test_is_supported() {
        assert!(WebSearchTool::is_supported());
    }

    #[test]
    fn test_not_a_tool_executor() {
        // WebSearchTool intentionally does NOT implement ToolExecutor.
        // It is a server-side tool. This test documents that design decision.
        // If someone tries to register it as a client tool, they'll get a
        // compile error because it doesn't impl ToolExecutor.
        let def = WebSearchTool::server_tool_definition();
        assert_eq!(def["name"], "web_search");
    }

    #[test]
    fn test_empty_domain_lists_omitted() {
        let def = WebSearchTool::server_tool_definition_with_domains(Some(&[]), Some(&[]));
        // Empty arrays should not be added
        assert!(def.get("allowed_domains").is_none());
        assert!(def.get("blocked_domains").is_none());
    }
}
