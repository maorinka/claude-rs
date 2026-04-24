//! `/batch` bundled-skill prompt.
//!
//! Port of TS `src/skills/bundled/batch.ts`. Orchestrates a
//! parallelizable change across the codebase by decomposing it
//! into 5–30 isolated worktree agents. The skill itself isn't
//! fully registered yet in Rust (requires AskUserQuestion +
//! worktree isolation wiring) but the prompt text is available
//! for future use and kept byte-stable for prompt-cache.
//!
//! TS template slots `${AGENT_TOOL_NAME}`, `${ASK_USER_QUESTION_TOOL_NAME}`,
//! `${ENTER_PLAN_MODE_TOOL_NAME}`, `${EXIT_PLAN_MODE_TOOL_NAME}`,
//! `${SKILL_TOOL_NAME}`, `${MIN_AGENTS}`, `${MAX_AGENTS}` are baked
//! in as literals to match the rendered TS output — same pattern
//! used for `loop.md`.

/// Worker-side instructions embedded inside the coordinator's
/// phase-2 section. Port of TS `batch.ts:12-17` `WORKER_INSTRUCTIONS`.
pub const BATCH_WORKER_INSTRUCTIONS: &str = "After you finish implementing the change:
1. **Simplify** — Invoke the `Skill` tool with `skill: \"simplify\"` to review and clean up your changes.
2. **Run unit tests** — Run the project's test suite (check for package.json scripts, Makefile targets, or common commands like `npm test`, `bun test`, `pytest`, `go test`). If tests fail, fix them.
3. **Test end-to-end** — Follow the e2e test recipe from the coordinator's prompt (below). If the recipe says to skip e2e for this unit, skip it.
4. **Commit and push** — Commit all changes with a clear message, push the branch, and create a PR with `gh pr create`. Use a descriptive title. If `gh` is not available or the push fails, note it in your final message.
5. **Report** — End with a single line: `PR: <url>` so the coordinator can track it. If no PR was created, end with `PR: none — <reason>`.";

/// Template for the coordinator prompt. Contains two placeholders:
/// `{{INSTRUCTION}}` (user's batch instruction) and
/// `{{WORKER_INSTRUCTIONS}}` (substituted with
/// [`BATCH_WORKER_INSTRUCTIONS`]). Use [`batch_prompt`] to render.
const BATCH_PROMPT_TEMPLATE: &str = include_str!("prompts/batch_skill.md");

/// Fallback message when the skill is invoked outside a git repo.
/// Port of TS `batch.ts:91` `NOT_A_GIT_REPO_MESSAGE`.
pub const BATCH_NOT_A_GIT_REPO_MESSAGE: &str =
    "This is not a git repository. The `/batch` command requires a git repo because it spawns agents in isolated git worktrees and creates PRs from each. Initialize a repo first, or run this from inside an existing one.";

/// Fallback message when the skill is invoked without args.
/// Port of TS `batch.ts:93-98` `MISSING_INSTRUCTION_MESSAGE`.
pub const BATCH_MISSING_INSTRUCTION_MESSAGE: &str =
    "Provide an instruction describing the batch change you want to make.

Examples:
  /batch migrate from react to vue
  /batch replace all uses of lodash with native equivalents
  /batch add type annotations to all untyped function parameters";

/// Build the full coordinator prompt for a given user instruction.
/// Port of TS `batch.ts:19-89` `buildPrompt(instruction)`.
pub fn batch_prompt(instruction: &str) -> String {
    BATCH_PROMPT_TEMPLATE
        .replace("{{INSTRUCTION}}", instruction)
        .replace("{{WORKER_INSTRUCTIONS}}", BATCH_WORKER_INSTRUCTIONS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_contains_three_phase_headers() {
        let p = batch_prompt("test instruction");
        assert!(p.contains("# Batch: Parallel Work Orchestration"));
        assert!(p.contains("## Phase 1: Research and Plan"));
        assert!(p.contains("## Phase 2: Spawn Workers"));
        assert!(p.contains("## Phase 3: Track Progress"));
    }

    #[test]
    fn prompt_substitutes_instruction_and_worker_block() {
        let p = batch_prompt("migrate from react to vue");
        assert!(p.contains("migrate from react to vue"));
        assert!(!p.contains("{{INSTRUCTION}}"));
        assert!(!p.contains("{{WORKER_INSTRUCTIONS}}"));
        // Worker instructions block is inlined.
        assert!(p.contains("PR: <url>"));
        assert!(p.contains("`Skill` tool with `skill: \"simplify\"`"));
    }

    #[test]
    fn prompt_uses_literal_tool_names() {
        let p = batch_prompt("x");
        // Tool name substitutions should be rendered, not template slots.
        assert!(p.contains("`EnterPlanMode`"));
        assert!(p.contains("`ExitPlanMode`"));
        assert!(p.contains("`Agent`"));
        assert!(p.contains("`AskUserQuestion`"));
    }

    #[test]
    fn min_max_agents_rendered_as_literals() {
        let p = batch_prompt("x");
        assert!(p.contains("5–30 self-contained units"));
        assert!(p.contains("closer to 5"));
        assert!(p.contains("closer to 30"));
    }

    #[test]
    fn fallback_messages_nonempty() {
        assert!(BATCH_NOT_A_GIT_REPO_MESSAGE.contains("git repository"));
        assert!(BATCH_MISSING_INSTRUCTION_MESSAGE.contains("/batch migrate"));
    }
}
