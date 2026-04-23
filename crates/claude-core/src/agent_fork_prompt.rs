//! Fork-subagent system-prompt sections for the Agent tool.
//!
//! Port of TS `src/tools/AgentTool/prompt.ts:81-121` —
//! `whenToForkSection` + `forkExamples`. The TS Agent tool
//! renders both sections only when fork is enabled (in-process
//! subagent that shares the parent's JS context + prompt cache);
//! they're suppressed in subprocess-only deployments.
//!
//! The Rust Agent tool (`claude-tools/src/agent_tool.rs`) ships
//! only the subprocess path today — fresh `claude-rs` process
//! per subagent, no shared context. Keeping the text in the
//! binary (even though no feature consumes it yet) means:
//! - the audit rollups have a place to point,
//! - when in-process fork lands, `build_agent_prompt()` can
//!   splice these constants in without a re-translation round-trip.

/// The `## When to fork` system-prompt section shown to the
/// model when fork is enabled. Verbatim port of TS
/// `whenToForkSection` at tools/AgentTool/prompt.ts:81-97.
///
/// The leading `\n\n` matches TS — the TS template concatenates
/// this directly after the `when NOT to use` block and needs
/// the gap so the heading starts on its own paragraph.
pub const AGENT_WHEN_TO_FORK_SECTION: &str = "

## When to fork

Fork yourself (omit `subagent_type`) when the intermediate tool output isn't worth keeping in your context. The criterion is qualitative — \"will I need this output again\" — not task size.
- **Research**: fork open-ended questions. If research can be broken into independent questions, launch parallel forks in one message. A fork beats a fresh subagent for this — it inherits context and shares your cache.
- **Implementation**: prefer to fork implementation work that requires more than a couple of edits. Do research before jumping to implementation.

Forks are cheap because they share your prompt cache. Don't set `model` on a fork — a different model can't reuse the parent's cache. Pass a short `name` (one or two words, lowercase) so the user can see the fork in the teams panel and steer it mid-run.

**Don't peek.** The tool result includes an `output_file` path — do not Read or tail it unless the user explicitly asks for a progress check. You get a completion notification; trust it. Reading the transcript mid-flight pulls the fork's tool noise into your context, which defeats the point of forking.

**Don't race.** After launching, you know nothing about what the fork found. Never fabricate or predict fork results in any format — not as prose, summary, or structured output. The notification arrives as a user-role message in a later turn; it is never something you write yourself. If the user asks a follow-up before the notification lands, tell them the fork is still running — give status, not a guess.

**Writing a fork prompt.** Since the fork inherits your context, the prompt is a *directive* — what to do, not what the situation is. Be specific about scope: what's in, what's out, what another agent is handling. Don't re-explain background.
";

/// Fork-aware examples block. Verbatim port of TS
/// `forkExamples` at tools/AgentTool/prompt.ts:115-120.
/// Shows three transcripts: a fork launch (survey question),
/// a mid-wait user follow-up, and a fresh-subagent (code-
/// reviewer) invocation.
///
/// TS interpolates `${AGENT_TOOL_NAME}` which always renders as
/// `Agent` — baked in here as the same literal the Rust Agent
/// tool registers itself under.
pub const AGENT_FORK_EXAMPLES: &str = "Example usage:

<example>
user: \"What's left on this branch before we can ship?\"
assistant: <thinking>Forking this — it's a survey question. I want the punch list, not the git output in my context.</thinking>
Agent({
  name: \"ship-audit\",
  description: \"Branch ship-readiness audit\",
  prompt: \"Audit what's left before this branch can ship. Check: uncommitted changes, commits ahead of main, whether tests exist, whether the GrowthBook gate is wired up, whether CI-relevant files changed. Report a punch list — done vs. missing. Under 200 words.\"
})
assistant: Ship-readiness audit running.
<commentary>
Turn ends here. The coordinator knows nothing about the findings yet. What follows is a SEPARATE turn — the notification arrives from outside, as a user-role message. It is not something the coordinator writes.
</commentary>
[later turn — notification arrives as user message]
assistant: Audit's back. Three blockers: no tests for the new prompt path, GrowthBook gate wired but not in build_flags.yaml, and one uncommitted file.
</example>

<example>
user: \"so is the gate wired up or not\"
<commentary>
User asks mid-wait. The audit fork was launched to answer exactly this, and it hasn't returned. The coordinator does not have this answer. Give status, not a fabricated result.
</commentary>
assistant: Still waiting on the audit — that's one of the things it's checking. Should land shortly.
</example>

<example>
user: \"Can you get a second opinion on whether this migration is safe?\"
assistant: <thinking>I'll ask the code-reviewer agent — it won't see my analysis, so it can give an independent read.</thinking>
<commentary>
A subagent_type is specified, so the agent starts fresh. It needs full context in the prompt. The briefing explains what to assess and why.
</commentary>
Agent({
  name: \"migration-review\",
  description: \"Independent migration review\",
  subagent_type: \"code-reviewer\",
  prompt: \"Review migration 0042_user_schema.sql for safety. Context: we're adding a NOT NULL column to a 50M-row table. Existing rows get a backfill default. I want a second opinion on whether the backfill approach is safe under concurrent writes — I've checked locking behavior but want independent verification. Report: is this safe, and if not, what specifically breaks?\"
})
</example>
";

/// Variant of the "Writing the prompt" lead-in rendered when
/// fork is enabled. TS template splices
/// `forkEnabled ? 'When spawning a fresh agent...' : ''` inline,
/// so callers needing the fork-aware phrasing concatenate this
/// ahead of the rest of the section. Port of
/// tools/AgentTool/prompt.ts:99-106 (fork-enabled branch).
pub const AGENT_WRITING_PROMPT_FORK_PREFIX: &str =
    "When spawning a fresh agent (with a `subagent_type`), it starts with zero context. ";

/// Fork-enabled phrasing of the "terse command-style prompts"
/// rule. TS flips `Terse` → `For fresh agents, terse` when fork
/// is on. Port of tools/AgentTool/prompt.ts:103.
pub const AGENT_WRITING_PROMPT_FORK_TERSE_LINE: &str =
    "For fresh agents, terse command-style prompts produce shallow, generic work.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn when_to_fork_header_and_anchors() {
        let s = AGENT_WHEN_TO_FORK_SECTION;
        assert!(s.starts_with("\n\n## When to fork"));
        assert!(s.contains("**Research**"));
        assert!(s.contains("**Implementation**"));
        assert!(s.contains("**Don't peek.**"));
        assert!(s.contains("**Don't race.**"));
        assert!(s.contains("**Writing a fork prompt.**"));
    }

    #[test]
    fn when_to_fork_keeps_cache_discipline_rule() {
        // The "don't set model on a fork" rule is load-bearing —
        // swapping models defeats cache sharing.
        assert!(AGENT_WHEN_TO_FORK_SECTION
            .contains("Don't set `model` on a fork — a different model can't reuse the parent's cache."));
    }

    #[test]
    fn fork_examples_renders_tool_name_as_agent() {
        // TS interpolates ${AGENT_TOOL_NAME}; the baked literal
        // must match the tool's registered name or the model
        // will call a non-existent tool.
        assert!(AGENT_FORK_EXAMPLES.contains("Agent({"));
        // No un-substituted template slots.
        assert!(!AGENT_FORK_EXAMPLES.contains("${AGENT_TOOL_NAME}"));
    }

    #[test]
    fn fork_examples_covers_three_canonical_transcripts() {
        let e = AGENT_FORK_EXAMPLES;
        // (1) survey-question fork → notification-arrives-later,
        // (2) mid-wait follow-up (give status not guess),
        // (3) fresh subagent (code-reviewer) invocation.
        assert!(e.contains("ship-audit"));
        assert!(e.contains("so is the gate wired up or not"));
        assert!(e.contains("subagent_type: \"code-reviewer\""));
    }

    #[test]
    fn writing_prompt_fork_prefix_is_prefix_not_full_section() {
        // The fork prefix is a sentence the TS template *splices
        // in* — not a standalone paragraph. It must end with a
        // trailing space so concatenation with the rest of the
        // section reads correctly.
        assert!(AGENT_WRITING_PROMPT_FORK_PREFIX.ends_with(". "));
    }

    #[test]
    fn fork_terse_line_mentions_fresh_agents() {
        assert!(AGENT_WRITING_PROMPT_FORK_TERSE_LINE.contains("For fresh agents"));
        assert!(AGENT_WRITING_PROMPT_FORK_TERSE_LINE.contains("shallow, generic work"));
    }
}
