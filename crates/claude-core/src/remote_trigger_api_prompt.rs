//! TS-equivalent prompt text for the `RemoteTrigger` API-proxy
//! tool.
//!
//! Port of TS `src/tools/RemoteTriggerTool/prompt.ts:1-16`. The
//! TS `RemoteTrigger` tool is an API proxy for the claude.ai
//! CCR trigger API (`list`/`get`/`create`/`update`/`run`
//! actions) — it hands the model a thin wrapper over the HTTP
//! endpoints and returns raw JSON.
//!
//! The Rust `RemoteTriggerTool` at
//! `crates/claude-tools/src/remote_trigger.rs` implements a
//! different surface today: it dispatches a prompt to the
//! cloud-execution environment via `RemoteClient::create_task()`.
//! The two surfaces aren't interchangeable — the API-proxy flavor
//! is what the `/schedule` bundled skill and
//! `schedule_remote_agents_prompt` in this crate expect to drive.
//!
//! Until the API-proxy variant ships on the Rust side, this
//! module hosts the canonical text so:
//! - `schedule_remote_agents_prompt` can reference a stable
//!   `RemoteTrigger` blurb without re-transcribing TS,
//! - the audit rollups have a concrete Rust home to point at.

/// Short, model-facing description. Verbatim from TS
/// `RemoteTriggerTool/prompt.ts:1-2` `DESCRIPTION`.
pub const REMOTE_TRIGGER_API_DESCRIPTION: &str =
    "Manage scheduled remote Claude Code agents (triggers) via the claude.ai CCR API. Auth is handled in-process — the token never reaches the shell.";

/// Full instructions the TS tool injects as its prompt body.
/// Verbatim from TS `RemoteTriggerTool/prompt.ts:4-14` `PROMPT`.
pub const REMOTE_TRIGGER_API_PROMPT: &str =
    "Call the claude.ai remote-trigger API. Use this instead of curl — the OAuth token is added automatically in-process and never exposed.

Actions:
- list: GET /v1/code/triggers
- get: GET /v1/code/triggers/{trigger_id}
- create: POST /v1/code/triggers (requires body)
- update: POST /v1/code/triggers/{trigger_id} (requires body, partial update)
- run: POST /v1/code/triggers/{trigger_id}/run

The response is the raw JSON from the API.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn description_mentions_in_process_auth() {
        assert!(REMOTE_TRIGGER_API_DESCRIPTION.contains("Auth is handled in-process"));
        assert!(REMOTE_TRIGGER_API_DESCRIPTION.contains("never reaches the shell"));
    }

    #[test]
    fn prompt_enumerates_all_five_actions() {
        for action in &[
            "list: GET /v1/code/triggers",
            "get: GET /v1/code/triggers/{trigger_id}",
            "create: POST /v1/code/triggers (requires body)",
            "update: POST /v1/code/triggers/{trigger_id} (requires body, partial update)",
            "run: POST /v1/code/triggers/{trigger_id}/run",
        ] {
            assert!(
                REMOTE_TRIGGER_API_PROMPT.contains(action),
                "prompt missing action `{action}`"
            );
        }
    }

    #[test]
    fn prompt_warns_against_curl() {
        // The `use this instead of curl` rule is load-bearing —
        // curl-based invocations leak the OAuth token to shell
        // history.
        assert!(REMOTE_TRIGGER_API_PROMPT.contains("instead of curl"));
    }

    #[test]
    fn prompt_ends_with_raw_json_contract() {
        assert!(REMOTE_TRIGGER_API_PROMPT.ends_with("raw JSON from the API."));
    }
}
