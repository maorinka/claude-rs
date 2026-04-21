//! Agent-creation system prompt — template + builder.
//!
//! Port of TS `components/agents/generateAgent.ts:26`
//! `AGENT_CREATION_SYSTEM_PROMPT`. TS fires a Sonnet query to
//! translate a user description into a new agent config
//! (identifier, whenToUse, systemPrompt). The Rust TUI has
//! hardcoded `AgentDefinition`s in
//! `crates/claude-tools/src/agents/definitions.rs` but no
//! dynamic creation/generation pathway — this module exposes
//! the prompt so when that caller lands it uses the verbatim
//! TS framing.
//!
//! TS interpolates `${AGENT_TOOL_NAME}` (usually `Agent`) into
//! the prompt's example blocks; the Rust port keeps the
//! template as-is in `.md` and substitutes at build-time via
//! [`agent_creation_system_prompt`].

use crate::tool_names::AGENT_TOOL_NAME;

/// Raw prompt template with `${AGENT_TOOL_NAME}` placeholders
/// intact. Prefer [`agent_creation_system_prompt`] for the
/// substituted form.
pub const AGENT_CREATION_SYSTEM_PROMPT_TEMPLATE: &str =
    include_str!("prompts/agent_creation.md");

/// Build the agent-creation system prompt with
/// `${AGENT_TOOL_NAME}` resolved to the current
/// `tool_names::AGENT_TOOL_NAME`.
pub fn agent_creation_system_prompt() -> String {
    AGENT_CREATION_SYSTEM_PROMPT_TEMPLATE.replace("${AGENT_TOOL_NAME}", AGENT_TOOL_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_has_placeholders() {
        assert!(AGENT_CREATION_SYSTEM_PROMPT_TEMPLATE.contains("${AGENT_TOOL_NAME}"));
    }

    #[test]
    fn built_prompt_has_no_placeholders() {
        let p = agent_creation_system_prompt();
        assert!(!p.contains("${AGENT_TOOL_NAME}"));
        assert!(p.contains(AGENT_TOOL_NAME));
    }

    #[test]
    fn prompt_contains_ts_anchor_phrases() {
        let p = agent_creation_system_prompt();
        assert!(p.contains("You are an elite AI agent architect"));
        assert!(p.contains("Extract Core Intent"));
        assert!(p.contains("Design Expert Persona"));
        assert!(p.contains("Your output must be a valid JSON object"));
        assert!(p.contains("identifier"));
        assert!(p.contains("whenToUse"));
        assert!(p.contains("systemPrompt"));
    }
}
