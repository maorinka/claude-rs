//! Agentic session-search prompts — system prompt + user message template.
//!
//! Port of TS `utils/agenticSessionSearch.ts:15-48` (system
//! prompt) and `:248-253` (user message template). The
//! LLM-powered session-finder query isn't yet ported; this
//! module exposes the verbatim TS strings so when the caller
//! lands it runs with identical framing.

/// System prompt for the session-search agentic query. Tells
/// the model to find relevant sessions by tag > title > branch
/// > summary/transcript, be inclusive, and return a JSON
/// > `{"relevant_indices": [...]}` response.
///
/// Port of TS `utils/agenticSessionSearch.ts:15-48`.
pub const SESSION_SEARCH_SYSTEM_PROMPT: &str = r#"Your goal is to find relevant sessions based on a user's search query.

You will be given a list of sessions with their metadata and a search query. Identify which sessions are most relevant to the query.

Each session may include:
- Title (display name or custom title)
- Tag (user-assigned category, shown as [tag: name] - users tag sessions with /tag command to categorize them)
- Branch (git branch name, shown as [branch: name])
- Summary (AI-generated summary)
- First message (beginning of the conversation)
- Transcript (excerpt of conversation content)

IMPORTANT: Tags are user-assigned labels that indicate the session's topic or category. If the query matches a tag exactly or partially, those sessions should be highly prioritized.

For each session, consider (in order of priority):
1. Exact tag matches (highest priority - user explicitly categorized this session)
2. Partial tag matches or tag-related terms
3. Title matches (custom titles or first message content)
4. Branch name matches
5. Summary and transcript content matches
6. Semantic similarity and related concepts

CRITICAL: Be VERY inclusive in your matching. Include sessions that:
- Contain the query term anywhere in any field
- Are semantically related to the query (e.g., "testing" matches sessions about "tests", "unit tests", "QA", etc.)
- Discuss topics that could be related to the query
- Have transcripts that mention the concept even in passing

When in doubt, INCLUDE the session. It's better to return too many results than too few. The user can easily scan through results, but missing relevant sessions is frustrating.

Return sessions ordered by relevance (most relevant first). If truly no sessions have ANY connection to the query, return an empty array - but this should be rare.

Respond with ONLY the JSON object, no markdown formatting:
{"relevant_indices": [2, 5, 0]}"#;

/// Build the user-message for the session-search query, with
/// the session list and the search query. Verbatim from TS
/// `utils/agenticSessionSearch.ts:248-253`.
pub fn session_search_user_message(session_list: &str, query: &str) -> String {
    format!(
        "Sessions:\n{session_list}\n\nSearch query: \"{query}\"\n\nFind the sessions that are most relevant to this query."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_prompt_contains_priority_rules() {
        assert!(SESSION_SEARCH_SYSTEM_PROMPT.contains("Exact tag matches"));
        assert!(SESSION_SEARCH_SYSTEM_PROMPT.contains("Be VERY inclusive"));
        assert!(SESSION_SEARCH_SYSTEM_PROMPT.contains(r#"{"relevant_indices":"#));
    }

    #[test]
    fn user_message_interpolates_fields() {
        let m = session_search_user_message("- session 1\n- session 2", "auth bug");
        assert!(m.contains("Sessions:\n- session 1\n- session 2"));
        assert!(m.contains("Search query: \"auth bug\""));
        assert!(m.ends_with("Find the sessions that are most relevant to this query."));
    }
}
