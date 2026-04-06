use std::path::Path;

use super::types::{HookCommand, HookEvent, HookMatcher, HooksSettings};
use tracing::debug;

// ============================================================================
// Match query resolution — determines what value to match against for each event
// ============================================================================

/// Given the hook event and the JSON input, extract the match query string.
///
/// This mirrors the TypeScript `getMatchingHooks` switch on `hookInput.hook_event_name`.
///
/// Returns `None` for events that have no match query (TeammateIdle, TaskCreated,
/// TaskCompleted, WorktreeCreate, WorktreeRemove, UserPromptSubmit, Stop, CwdChanged).
pub fn resolve_match_query(event: &HookEvent, hook_input: &serde_json::Value) -> Option<String> {
    match event {
        // tool_name based events
        HookEvent::PreToolUse
        | HookEvent::PostToolUse
        | HookEvent::PostToolUseFailure
        | HookEvent::PermissionRequest
        | HookEvent::PermissionDenied => hook_input
            .get("tool_name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),

        // source-based events
        HookEvent::SessionStart | HookEvent::ConfigChange => hook_input
            .get("source")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),

        // trigger-based events
        HookEvent::Setup | HookEvent::PreCompact | HookEvent::PostCompact => hook_input
            .get("trigger")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),

        // notification_type
        HookEvent::Notification => hook_input
            .get("notification_type")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),

        // reason-based
        HookEvent::SessionEnd => hook_input
            .get("reason")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),

        // error string
        HookEvent::StopFailure => hook_input
            .get("error")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),

        // agent_type
        HookEvent::SubagentStart | HookEvent::SubagentStop => hook_input
            .get("agent_type")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),

        // mcp_server_name
        HookEvent::Elicitation | HookEvent::ElicitationResult => hook_input
            .get("mcp_server_name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),

        // load_reason
        HookEvent::InstructionsLoaded => hook_input
            .get("load_reason")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),

        // file_path -> basename
        HookEvent::FileChanged => hook_input
            .get("file_path")
            .and_then(|v| v.as_str())
            .map(|p| {
                Path::new(p)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| p.to_string())
            }),

        // Events with no match query
        HookEvent::TeammateIdle
        | HookEvent::TaskCreated
        | HookEvent::TaskCompleted
        | HookEvent::UserPromptSubmit
        | HookEvent::Stop
        | HookEvent::WorktreeCreate
        | HookEvent::WorktreeRemove
        | HookEvent::CwdChanged => None,
    }
}

// ============================================================================
// Pattern matching — simple string, pipe-separated, wildcard, and regex
// ============================================================================

/// Check if a match query matches a hook matcher pattern.
///
/// Supports:
/// - Empty or "*" -> matches everything
/// - Simple string for exact match (e.g., "Write")
/// - Pipe-separated list for multiple exact matches (e.g., "Write|Edit")
/// - Regex pattern (e.g., "^Write.*", ".*", "^(Write|Edit)$")
///
/// Mirrors the TypeScript `matchesPattern()`.
pub fn matches_pattern(match_query: &str, matcher: &str) -> bool {
    if matcher.is_empty() || matcher == "*" {
        return true;
    }

    // Check if it's a simple string or pipe-separated list (no regex special chars except |)
    let is_simple = matcher
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '|');

    if is_simple {
        if matcher.contains('|') {
            // Handle pipe-separated exact matches
            return matcher.split('|').any(|p| p.trim() == match_query);
        }
        // Simple exact match
        return match_query == matcher;
    }

    // Otherwise treat as regex
    match regex::Regex::new(matcher) {
        Ok(re) => re.is_match(match_query),
        Err(_) => {
            debug!("Invalid regex pattern in hook matcher: {}", matcher);
            false
        }
    }
}

// ============================================================================
// `if` condition evaluation
// ============================================================================

/// Evaluate a hook `if` condition against the current hook event and input.
///
/// The `if` condition uses permission rule syntax: "ToolName" or "ToolName(ruleContent)".
/// Only supported for tool-based events (PreToolUse, PostToolUse, PostToolUseFailure,
/// PermissionRequest). For all other events the hook is skipped (fail-safe to match TS).
///
/// Returns `true` if the hook should run, `false` if it should be skipped.
pub fn evaluate_if_condition(
    if_condition: &str,
    event: &HookEvent,
    hook_input: &serde_json::Value,
) -> bool {
    use crate::permissions::types::PermissionRuleValue;

    let is_tool_event = matches!(
        event,
        HookEvent::PreToolUse
            | HookEvent::PostToolUse
            | HookEvent::PostToolUseFailure
            | HookEvent::PermissionRequest
    );

    if !is_tool_event {
        debug!(
            "Hook if condition {:?} cannot be evaluated for non-tool event {}; skipping hook",
            if_condition, event
        );
        return false;
    }

    let tool_name = match hook_input.get("tool_name").and_then(|v| v.as_str()) {
        Some(n) => n,
        None => return false,
    };

    let parsed = PermissionRuleValue::from_string(if_condition);

    // Tool name must match (PermissionRuleValue::from_string already normalises legacy names)
    if parsed.tool_name != tool_name {
        return false;
    }

    // No rule content -> tool-level match is sufficient
    let rule_content = match parsed.rule_content {
        Some(ref c) => c.clone(),
        None => return true,
    };

    // Extract the primary input string for this tool.
    // Bash uses "command"; file tools use "file_path"; fall back to JSON dump.
    let tool_input = hook_input.get("tool_input");
    let command_str: Option<String> = tool_input.and_then(|inp| {
        inp.get("command")
            .or_else(|| inp.get("file_path"))
            .or_else(|| inp.get("path"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| inp.as_str().map(|s| s.to_string()))
    });

    let command_str = match command_str {
        // Fail-safe: if we cannot extract a command, run the hook (matches TS "too complex" path)
        None => return true,
        Some(s) => s,
    };

    // For compound shell commands split on common operators and check if ANY
    // sub-command matches. This mirrors the TS "fail-safe: run hook if too complex" logic.
    command_str
        .split(|c: char| c == ';' || c == '&' || c == '|')
        .any(|part| glob_match(&rule_content, part.trim()))
}

/// Simple glob pattern matching: `*` matches any sequence of characters.
/// Handles single `*` wildcards. Mirrors TS permission rule content matching.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    if pattern == "*" || pattern.is_empty() {
        return true;
    }
    if !pattern.contains('*') {
        // Prefix match: "git " matches "git status", etc.
        return text.starts_with(pattern) || text == pattern;
    }
    // Split on the first `*` only (common case: "git *", "npm *")
    let parts: Vec<&str> = pattern.splitn(2, '*').collect();
    match parts.as_slice() {
        [prefix, suffix] => {
            text.starts_with(prefix) && (suffix.is_empty() || text.ends_with(suffix))
        }
        _ => text == pattern,
    }
}

// ============================================================================
// Get matching hooks from settings
// ============================================================================

/// A hook paired with its source context for deduplication and execution.
#[derive(Clone, Debug)]
pub struct MatchedHook {
    pub hook: HookCommand,
    pub hook_source: String,
}

/// Build a dedup key for a matched hook.
///
/// Settings-file hooks share the empty prefix, so the same command from
/// user/project/local collapses to one. The `if` condition is part of the
/// key so hooks with different conditions don't collapse.
fn hook_dedup_key(hook: &HookCommand) -> String {
    let if_cond = hook.if_condition().unwrap_or("");
    match hook {
        HookCommand::Command(h) => {
            let shell = match &h.shell {
                Some(s) => format!("{:?}", s),
                None => "bash".to_string(),
            };
            format!("command\0{}\0{}\0{}", shell, h.command, if_cond)
        }
        HookCommand::Prompt(h) => format!("prompt\0{}\0{}", h.prompt, if_cond),
        HookCommand::Http(h) => format!("http\0{}\0{}", h.url, if_cond),
        HookCommand::Agent(h) => format!("agent\0{}\0{}", h.prompt, if_cond),
    }
}

/// Get hook commands that match the given event and input.
///
/// Steps:
/// 1. Look up all matchers for the event
/// 2. Filter by match query (resolve_match_query)
/// 3. Extract individual hooks
/// 4. Deduplicate by command/prompt/url + if condition
/// 5. Filter HTTP hooks out of SessionStart/Setup events
pub fn get_matching_hooks(
    settings: &HooksSettings,
    event: &HookEvent,
    hook_input: &serde_json::Value,
) -> Vec<MatchedHook> {
    let matchers = match settings.get(event) {
        Some(m) => m,
        None => return Vec::new(),
    };

    if matchers.is_empty() {
        return Vec::new();
    }

    let match_query = resolve_match_query(event, hook_input);

    debug!(
        "Getting matching hook commands for {} with query: {:?}",
        event, match_query
    );
    debug!("Found {} hook matchers in settings", matchers.len());

    // Filter matchers by match query
    let filtered_matchers: Vec<&HookMatcher> = if let Some(ref query) = match_query {
        matchers
            .iter()
            .filter(|m| {
                m.matcher
                    .as_ref()
                    .map(|pat| matches_pattern(query, pat))
                    .unwrap_or(true)
            })
            .collect()
    } else {
        matchers.iter().collect()
    };

    // Extract hooks from matchers
    let matched_hooks: Vec<MatchedHook> = filtered_matchers
        .into_iter()
        .flat_map(|matcher| {
            matcher.hooks.iter().map(move |hook| MatchedHook {
                hook: hook.clone(),
                hook_source: "settings".to_string(),
            })
        })
        .collect();

    // Deduplicate hooks by their dedup key (last occurrence wins, matching TS Map behavior).
    // We preserve insertion order by tracking seen keys and replacing in-place.
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut unique_hooks: Vec<MatchedHook> = Vec::new();
    for matched in &matched_hooks {
        let key = hook_dedup_key(&matched.hook);
        if let Some(&idx) = seen.get(&key) {
            // Last occurrence wins — replace the earlier entry.
            unique_hooks[idx] = matched.clone();
        } else {
            seen.insert(key, unique_hooks.len());
            unique_hooks.push(matched.clone());
        }
    }

    // Filter hooks by their `if` condition (permission rule syntax, e.g. "Bash(git *)").
    unique_hooks.retain(|matched| {
        if let Some(cond) = matched.hook.if_condition() {
            let keep = evaluate_if_condition(cond, event, hook_input);
            if !keep {
                debug!(
                    "Skipping hook due to if condition {:?} not matching",
                    cond
                );
            }
            keep
        } else {
            true
        }
    });

    // Filter out HTTP hooks for SessionStart/Setup events
    // (HTTP hooks deadlock in headless mode for these events)
    if *event == HookEvent::SessionStart || *event == HookEvent::Setup {
        unique_hooks.retain(|h| {
            if matches!(h.hook, HookCommand::Http(_)) {
                debug!(
                    "Skipping HTTP hook — HTTP hooks are not supported for {}",
                    event
                );
                false
            } else {
                true
            }
        });
    }

    debug!(
        "Matched {} unique hooks for query {:?} ({} before deduplication)",
        unique_hooks.len(),
        match_query.as_deref().unwrap_or("no match query"),
        matched_hooks.len()
    );

    unique_hooks
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_matches_pattern_empty() {
        assert!(matches_pattern("Write", ""));
        assert!(matches_pattern("anything", "*"));
    }

    #[test]
    fn test_matches_pattern_exact() {
        assert!(matches_pattern("Write", "Write"));
        assert!(!matches_pattern("Write", "Edit"));
    }

    #[test]
    fn test_matches_pattern_pipe_separated() {
        assert!(matches_pattern("Write", "Write|Edit"));
        assert!(matches_pattern("Edit", "Write|Edit"));
        assert!(!matches_pattern("Bash", "Write|Edit"));
    }

    #[test]
    fn test_matches_pattern_regex() {
        assert!(matches_pattern("Write", "^Write.*"));
        assert!(matches_pattern("WriteFoo", "^Write.*"));
        assert!(!matches_pattern("Edit", "^Write.*"));
        assert!(matches_pattern("anything", ".*"));
    }

    #[test]
    fn test_resolve_match_query_pre_tool_use() {
        let input = serde_json::json!({
            "hook_event_name": "PreToolUse",
            "tool_name": "Write"
        });
        assert_eq!(
            resolve_match_query(&HookEvent::PreToolUse, &input),
            Some("Write".to_string())
        );
    }

    #[test]
    fn test_resolve_match_query_notification() {
        let input = serde_json::json!({
            "hook_event_name": "Notification",
            "notification_type": "task_complete"
        });
        assert_eq!(
            resolve_match_query(&HookEvent::Notification, &input),
            Some("task_complete".to_string())
        );
    }

    #[test]
    fn test_resolve_match_query_file_changed_basename() {
        let input = serde_json::json!({
            "hook_event_name": "FileChanged",
            "file_path": "/home/user/project/src/main.rs"
        });
        assert_eq!(
            resolve_match_query(&HookEvent::FileChanged, &input),
            Some("main.rs".to_string())
        );
    }

    #[test]
    fn test_resolve_match_query_no_query() {
        let input = serde_json::json!({ "hook_event_name": "Stop" });
        assert_eq!(resolve_match_query(&HookEvent::Stop, &input), None);
    }
}
