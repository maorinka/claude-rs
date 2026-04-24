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
pub const AGENT_CREATION_SYSTEM_PROMPT_TEMPLATE: &str = include_str!("prompts/agent_creation.md");

/// Build the agent-creation system prompt with
/// `${AGENT_TOOL_NAME}` resolved to the current
/// `tool_names::AGENT_TOOL_NAME`.
pub fn agent_creation_system_prompt() -> String {
    AGENT_CREATION_SYSTEM_PROMPT_TEMPLATE.replace("${AGENT_TOOL_NAME}", AGENT_TOOL_NAME)
}

/// Conditional addon to the agent-creation system prompt when
/// the user's request touches memory/learn/remember semantics
/// (or the domain obviously benefits from cross-session memory).
/// Port of TS `AGENT_MEMORY_INSTRUCTIONS` at
/// components/agents/generateAgent.ts:100-119. TS appends this
/// to the base prompt only when the heuristic fires; the Rust
/// port exposes it as a standalone const so callers concatenate
/// or skip per their own detection.
pub const AGENT_MEMORY_INSTRUCTIONS: &str = "

7. **Agent Memory Instructions**: If the user mentions \"memory\", \"remember\", \"learn\", \"persist\", or similar concepts, OR if the agent would benefit from building up knowledge across conversations (e.g., code reviewers learning patterns, architects learning codebase structure, etc.), include domain-specific memory update instructions in the systemPrompt.

   Add a section like this to the systemPrompt, tailored to the agent's specific domain:

   \"**Update your agent memory** as you discover [domain-specific items]. This builds up institutional knowledge across conversations. Write concise notes about what you found and where.

   Examples of what to record:
   - [domain-specific item 1]
   - [domain-specific item 2]
   - [domain-specific item 3]\"

   Examples of domain-specific memory instructions:
   - For a code-reviewer: \"Update your agent memory as you discover code patterns, style conventions, common issues, and architectural decisions in this codebase.\"
   - For a test-runner: \"Update your agent memory as you discover test patterns, common failure modes, flaky tests, and testing best practices.\"
   - For an architect: \"Update your agent memory as you discover codepaths, library locations, key architectural decisions, and component relationships.\"
   - For a documentation writer: \"Update your agent memory as you discover documentation patterns, API structures, and terminology conventions.\"

   The memory instructions should be specific to what the agent would naturally learn while performing its core tasks.
";

/// Build the user-facing prompt that asks Sonnet to emit a new
/// agent config. Port of TS `components/agents/generateAgent.ts:133`.
///
/// - `user_prompt` is the free-form description of the agent.
/// - `existing_list` is the pre-formatted "Existing agents:
///   name — description" block that TS computes from the current
///   agent directory. Pass an empty string when there are none.
pub fn agent_generation_user_prompt(user_prompt: &str, existing_list: &str) -> String {
    format!(
        "Create an agent configuration based on this request: \"{user_prompt}\".{existing_list}\n  Return ONLY the JSON object, no other text."
    )
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

    #[test]
    fn memory_instructions_have_step_7_and_domain_examples() {
        let m = AGENT_MEMORY_INSTRUCTIONS;
        assert!(m.contains("7. **Agent Memory Instructions**"));
        assert!(m.contains("Update your agent memory"));
        // All four domain examples.
        assert!(m.contains("For a code-reviewer:"));
        assert!(m.contains("For a test-runner:"));
        assert!(m.contains("For an architect:"));
        assert!(m.contains("For a documentation writer:"));
    }

    #[test]
    fn user_prompt_quotes_description_and_ends_with_json_only() {
        let p = agent_generation_user_prompt("a security reviewer", "");
        assert!(p.contains("\"a security reviewer\""));
        assert!(p.ends_with("Return ONLY the JSON object, no other text."));
    }

    #[test]
    fn user_prompt_splices_existing_list_verbatim() {
        let existing = "\nExisting agents: reviewer — reviews code, tester — runs tests";
        let p = agent_generation_user_prompt("another reviewer", existing);
        assert!(p.contains("reviewer — reviews code"));
        // The TS template inserts `${existingList}` right after the
        // request sentence with no separator of its own — the
        // caller pre-formats the leading newline / "Existing"
        // wording.
        assert!(p.contains("\".\nExisting agents:"));
    }
}
