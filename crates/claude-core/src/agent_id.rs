//! Deterministic agent + request ID formatting/parsing.
//!
//! Port of TS `src/utils/agentId.ts`. Agent IDs are
//! `{agent_name}@{team_name}`; request IDs are
//! `{request_type}-{timestamp_ms}@{agent_id}`. Used by the
//! swarm/teammate subsystem for deterministic reconnect + message
//! routing. Agent names must not contain `@` — callers should strip
//! it upstream (TS uses `sanitizeAgentName`).

use chrono::Utc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedAgentId {
    pub agent_name: String,
    pub team_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedRequestId {
    pub request_type: String,
    pub timestamp_ms: i64,
    pub agent_id: String,
}

/// Format an agent ID as `{agent_name}@{team_name}`.
pub fn format_agent_id(agent_name: &str, team_name: &str) -> String {
    format!("{agent_name}@{team_name}")
}

/// Split an agent ID on the first `@`. Returns `None` if the input
/// has no `@`.
pub fn parse_agent_id(agent_id: &str) -> Option<ParsedAgentId> {
    let (agent, team) = agent_id.split_once('@')?;
    Some(ParsedAgentId {
        agent_name: agent.to_string(),
        team_name: team.to_string(),
    })
}

/// Build a request ID as `{request_type}-{now_ms}@{agent_id}`.
/// The timestamp is drawn from `chrono::Utc::now()` so tests can
/// mock it by overriding the system clock; callers wanting a fixed
/// stamp should use [`format_request_id`].
pub fn generate_request_id(request_type: &str, agent_id: &str) -> String {
    format_request_id(request_type, Utc::now().timestamp_millis(), agent_id)
}

/// Build a request ID with an explicit timestamp (Unix ms).
pub fn format_request_id(request_type: &str, timestamp_ms: i64, agent_id: &str) -> String {
    format!("{request_type}-{timestamp_ms}@{agent_id}")
}

/// Parse a request ID. Returns `None` when the input does not split
/// into `{prefix}@{agent_id}` or when `prefix` has no `-` separating
/// the request type from the timestamp, or when the timestamp isn't
/// a base-10 integer.
pub fn parse_request_id(request_id: &str) -> Option<ParsedRequestId> {
    let (prefix, agent_id) = request_id.split_once('@')?;
    let last_dash = prefix.rfind('-')?;
    let request_type = &prefix[..last_dash];
    let timestamp_str = &prefix[last_dash + 1..];
    let timestamp_ms: i64 = timestamp_str.parse().ok()?;
    Some(ParsedRequestId {
        request_type: request_type.to_string(),
        timestamp_ms,
        agent_id: agent_id.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_agent_id_joins_with_at() {
        assert_eq!(format_agent_id("lead", "proj"), "lead@proj");
    }

    #[test]
    fn parse_agent_id_splits_on_first_at() {
        let parsed = parse_agent_id("lead@proj").unwrap();
        assert_eq!(parsed.agent_name, "lead");
        assert_eq!(parsed.team_name, "proj");
    }

    #[test]
    fn parse_agent_id_no_at_returns_none() {
        assert!(parse_agent_id("lead").is_none());
    }

    #[test]
    fn parse_agent_id_empty_team_ok() {
        let parsed = parse_agent_id("lead@").unwrap();
        assert_eq!(parsed.agent_name, "lead");
        assert_eq!(parsed.team_name, "");
    }

    #[test]
    fn format_request_id_shape() {
        assert_eq!(
            format_request_id("shutdown", 1_700_000_000_000, "r@team"),
            "shutdown-1700000000000@r@team"
        );
    }

    #[test]
    fn parse_request_id_roundtrip() {
        let id = format_request_id("shutdown", 1_700_000_000_000, "r@team");
        let parsed = parse_request_id(&id).unwrap();
        assert_eq!(parsed.request_type, "shutdown");
        assert_eq!(parsed.timestamp_ms, 1_700_000_000_000);
        assert_eq!(parsed.agent_id, "r@team");
    }

    #[test]
    fn parse_request_id_handles_dashes_in_type() {
        let id = "plan-approval-1234567890@a@t";
        let parsed = parse_request_id(id).unwrap();
        assert_eq!(parsed.request_type, "plan-approval");
        assert_eq!(parsed.timestamp_ms, 1_234_567_890);
        assert_eq!(parsed.agent_id, "a@t");
    }

    #[test]
    fn parse_request_id_no_at_is_none() {
        assert!(parse_request_id("shutdown-123").is_none());
    }

    #[test]
    fn parse_request_id_no_dash_in_prefix_is_none() {
        assert!(parse_request_id("shutdown@a@t").is_none());
    }

    #[test]
    fn parse_request_id_non_numeric_timestamp_is_none() {
        assert!(parse_request_id("shutdown-abc@a@t").is_none());
    }

    #[test]
    fn generate_request_id_uses_current_time_millis() {
        let id = generate_request_id("shutdown", "a@t");
        let parsed = parse_request_id(&id).expect("parses");
        let now = Utc::now().timestamp_millis();
        assert!(
            (now - parsed.timestamp_ms).abs() < 5_000,
            "generated id timestamp diverged from now: {parsed:?}"
        );
    }
}
