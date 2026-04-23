//! Plan mode workflow prompts (5-phase full + iterative
//! interview variant).
//!
//! Port of TS `utils/messages.ts:3207-3378` — the two variants
//! of the plan-mode system prompt:
//! - **5-phase workflow** (default): Phase 1 explore agents →
//!   Phase 2 design agents → Phase 3 review → Phase 4 final
//!   plan (one of four variants) → Phase 5 call `ExitPlanMode`.
//! - **Interview workflow**: pair-planning loop
//!   (explore → update → ask).
//!
//! Plan mode in the Rust port today uses the simpler enter/exit
//! in `claude-tools/src/plan_mode.rs`. This module parks the
//! full workflow text so when subagent dispatch + the iterative
//! planning UX land, callers can splice them in without
//! re-translating.
//!
//! # Tool-name literals
//!
//! TS template interpolations are baked as literals:
//! - `${FileEditTool.name}` → `Edit`
//! - `${FileWriteTool.name}` → `Write`
//! - `${ASK_USER_QUESTION_TOOL_NAME}` → `AskUserQuestion`
//! - `${ExitPlanModeV2Tool.name}` → `ExitPlanMode`
//! - `${EXPLORE_AGENT.agentType}` → `Explore`
//! - `${PLAN_AGENT.agentType}` → `Plan`
//! - `${agentCount}` / `${exploreAgentCount}` → caller-provided

/// Plan mode 5-phase workflow system prompt. Port of TS
/// `getPlanModeV2Instructions` at
/// `utils/messages.ts:3207-3292`.
///
/// # Fields
///
/// - `plan_exists`: if true, the existing-file branch;
///   otherwise the new-file branch.
/// - `plan_file_path`: absolute path of the plan file.
/// - `explore_agent_count`: max parallel Explore agents (TS
///   reads this from `getPlanModeV2ExploreAgentCount()`).
/// - `plan_agent_count`: max parallel Plan agents (TS
///   `getPlanModeV2AgentCount()`).
/// - `phase4_section`: one of
///   [`plan_mode::PLAN_PHASE4_CONTROL`] / `..._TRIM` / `..._CUT`
///   / `..._CAP` — the caller picks the variant per the
///   `plan-mode-budget` GrowthBook flag.
pub struct PlanModeV2Inputs<'a> {
    pub plan_exists: bool,
    pub plan_file_path: &'a str,
    pub explore_agent_count: u32,
    pub plan_agent_count: u32,
    pub phase4_section: &'a str,
}

fn plan_file_info(plan_exists: bool, plan_file_path: &str) -> String {
    if plan_exists {
        format!("A plan file already exists at {plan_file_path}. You can read it and make incremental edits using the Edit tool.")
    } else {
        format!("No plan file exists yet. You should create your plan at {plan_file_path} using the Write tool.")
    }
}

/// Build the full 5-phase plan-mode system prompt.
pub fn plan_mode_v2_instructions(inputs: &PlanModeV2Inputs<'_>) -> String {
    let info = plan_file_info(inputs.plan_exists, inputs.plan_file_path);
    let exp = inputs.explore_agent_count;
    let cnt = inputs.plan_agent_count;
    let multi_agent_block = if cnt > 1 {
        format!(
            "- **Multiple agents**: Use up to {cnt} agents for complex tasks that benefit from different perspectives\n\n\
             Examples of when to use multiple agents:\n\
             - The task touches multiple parts of the codebase\n\
             - It's a large refactor or architectural change\n\
             - There are many edge cases to consider\n\
             - You'd benefit from exploring different approaches\n\n\
             Example perspectives by task type:\n\
             - New feature: simplicity vs performance vs maintainability\n\
             - Bug fix: root cause vs workaround vs prevention\n\
             - Refactoring: minimal change vs clean architecture\n"
        )
    } else {
        String::new()
    };
    let phase4 = inputs.phase4_section;

    format!(
        "Plan mode is active. The user indicated that they do not want you to execute yet -- you MUST NOT make any edits (with the exception of the plan file mentioned below), run any non-readonly tools (including changing configs or making commits), or otherwise make any changes to the system. This supercedes any other instructions you have received.\n\n\
         ## Plan File Info:\n\
         {info}\n\
         You should build your plan incrementally by writing to or editing this file. NOTE that this is the only file you are allowed to edit - other than this you are only allowed to take READ-ONLY actions.\n\n\
         ## Plan Workflow\n\n\
         ### Phase 1: Initial Understanding\n\
         Goal: Gain a comprehensive understanding of the user's request by reading through code and asking them questions. Critical: In this phase you should only use the Explore subagent type.\n\n\
         1. Focus on understanding the user's request and the code associated with their request. Actively search for existing functions, utilities, and patterns that can be reused — avoid proposing new code when suitable implementations already exist.\n\n\
         2. **Launch up to {exp} Explore agents IN PARALLEL** (single message, multiple tool calls) to efficiently explore the codebase.\n   \
         - Use 1 agent when the task is isolated to known files, the user provided specific file paths, or you're making a small targeted change.\n   \
         - Use multiple agents when: the scope is uncertain, multiple areas of the codebase are involved, or you need to understand existing patterns before planning.\n   \
         - Quality over quantity - {exp} agents maximum, but you should try to use the minimum number of agents necessary (usually just 1)\n   \
         - If using multiple agents: Provide each agent with a specific search focus or area to explore. Example: One agent searches for existing implementations, another explores related components, a third investigating testing patterns\n\n\
         ### Phase 2: Design\n\
         Goal: Design an implementation approach.\n\n\
         Launch Plan agent(s) to design the implementation based on the user's intent and your exploration results from Phase 1.\n\n\
         You can launch up to {cnt} agent(s) in parallel.\n\n\
         **Guidelines:**\n\
         - **Default**: Launch at least 1 Plan agent for most tasks - it helps validate your understanding and consider alternatives\n\
         - **Skip agents**: Only for truly trivial tasks (typo fixes, single-line changes, simple renames)\n\
         {multi_agent_block}\
         In the agent prompt:\n\
         - Provide comprehensive background context from Phase 1 exploration including filenames and code path traces\n\
         - Describe requirements and constraints\n\
         - Request a detailed implementation plan\n\n\
         ### Phase 3: Review\n\
         Goal: Review the plan(s) from Phase 2 and ensure alignment with the user's intentions.\n\
         1. Read the critical files identified by agents to deepen your understanding\n\
         2. Ensure that the plans align with the user's original request\n\
         3. Use AskUserQuestion to clarify any remaining questions with the user\n\n\
         {phase4}\n\n\
         ### Phase 5: Call ExitPlanMode\n\
         At the very end of your turn, once you have asked the user questions and are happy with your final plan file - you should always call ExitPlanMode to indicate to the user that you are done planning.\n\
         This is critical - your turn should only end with either using the AskUserQuestion tool OR calling ExitPlanMode. Do not stop unless it's for these 2 reasons\n\n\
         **Important:** Use AskUserQuestion ONLY to clarify requirements or choose between approaches. Use ExitPlanMode to request plan approval. Do NOT ask about plan approval in any other way - no text questions, no AskUserQuestion. Phrases like \"Is this plan okay?\", \"Should I proceed?\", \"How does this plan look?\", \"Any changes before we start?\", or similar MUST use ExitPlanMode.\n\n\
         NOTE: At any point in time through this workflow you should feel free to ask the user questions or clarifications using the AskUserQuestion tool. Don't make large assumptions about user intent. The goal is to present a well researched plan to the user, and tie any loose ends before implementation begins."
    )
}

/// Plan mode iterative interview system prompt. Port of TS
/// `getPlanModeInterviewInstructions` at
/// `utils/messages.ts:3323-3378`.
///
/// - `plan_exists` / `plan_file_path`: same semantics as 5-phase.
/// - `read_only_tools_list`: pre-rendered list of read-only tool
///   names (TS `getReadOnlyToolNames()` returns e.g.
///   `"Read, Grep, Glob"`).
/// - `explore_plan_agents_enabled`: when true, appends the TS
///   `areExplorePlanAgentsEnabled()` sentence about using the
///   Explore agent for parallelizing complex searches.
pub fn plan_mode_interview_instructions(
    plan_exists: bool,
    plan_file_path: &str,
    read_only_tools_list: &str,
    explore_plan_agents_enabled: bool,
) -> String {
    let info = plan_file_info(plan_exists, plan_file_path);
    let explore_sentence = if explore_plan_agents_enabled {
        " You can use the Explore agent type to parallelize complex searches without filling your context, though for straightforward queries direct tools are simpler."
    } else {
        ""
    };

    format!(
        "Plan mode is active. The user indicated that they do not want you to execute yet -- you MUST NOT make any edits (with the exception of the plan file mentioned below), run any non-readonly tools (including changing configs or making commits), or otherwise make any changes to the system. This supercedes any other instructions you have received.\n\n\
         ## Plan File Info:\n\
         {info}\n\n\
         ## Iterative Planning Workflow\n\n\
         You are pair-planning with the user. Explore the code to build context, ask the user questions when you hit decisions you can't make alone, and write your findings into the plan file as you go. The plan file (above) is the ONLY file you may edit — it starts as a rough skeleton and gradually becomes the final plan.\n\n\
         ### The Loop\n\n\
         Repeat this cycle until the plan is complete:\n\n\
         1. **Explore** — Use {read_only_tools_list} to read code. Look for existing functions, utilities, and patterns to reuse.{explore_sentence}\n\
         2. **Update the plan file** — After each discovery, immediately capture what you learned. Don't wait until the end.\n\
         3. **Ask the user** — When you hit an ambiguity or decision you can't resolve from code alone, use AskUserQuestion. Then go back to step 1.\n\n\
         ### First Turn\n\n\
         Start by quickly scanning a few key files to form an initial understanding of the task scope. Then write a skeleton plan (headers and rough notes) and ask the user your first round of questions. Don't explore exhaustively before engaging the user.\n\n\
         ### Asking Good Questions\n\n\
         - Never ask what you could find out by reading the code\n\
         - Batch related questions together (use multi-question AskUserQuestion calls)\n\
         - Focus on things only the user can answer: requirements, preferences, tradeoffs, edge case priorities\n\
         - Scale depth to the task — a vague feature request needs many rounds; a focused bug fix may need one or none\n\n\
         ### Plan File Structure\n\
         Your plan file should be divided into clear sections using markdown headers, based on the request. Fill out these sections as you go.\n\
         - Begin with a **Context** section: explain why this change is being made — the problem or need it addresses, what prompted it, and the intended outcome\n\
         - Include only your recommended approach, not all alternatives\n\
         - Ensure that the plan file is concise enough to scan quickly, but detailed enough to execute effectively\n\
         - Include the paths of critical files to be modified\n\
         - Reference existing functions and utilities you found that should be reused, with their file paths\n\
         - Include a verification section describing how to test the changes end-to-end (run the code, use MCP tools, run tests)\n\n\
         ### When to Converge\n\n\
         Your plan is ready when you've addressed all ambiguities and it covers: what to change, which files to modify, what existing code to reuse (with file paths), and how to verify the changes. Call ExitPlanMode when the plan is ready for approval.\n\n\
         ### Ending Your Turn\n\n\
         Your turn should only end by either:\n\
         - Using AskUserQuestion to gather more information\n\
         - Calling ExitPlanMode when the plan is ready for approval\n\n\
         **Important:** Use ExitPlanMode to request plan approval. Do NOT ask about plan approval via text or AskUserQuestion."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_v2_inputs<'a>() -> PlanModeV2Inputs<'a> {
        PlanModeV2Inputs {
            plan_exists: false,
            plan_file_path: "/plan.md",
            explore_agent_count: 3,
            plan_agent_count: 2,
            phase4_section: "### Phase 4: Final Plan\nShort body.",
        }
    }

    #[test]
    fn v2_has_five_phases() {
        let p = plan_mode_v2_instructions(&default_v2_inputs());
        for phase in &[
            "### Phase 1: Initial Understanding",
            "### Phase 2: Design",
            "### Phase 3: Review",
            "### Phase 4: Final Plan",
            "### Phase 5: Call ExitPlanMode",
        ] {
            assert!(p.contains(phase), "missing {phase}");
        }
    }

    #[test]
    fn v2_interpolates_counts_in_phase1_and_phase2() {
        let p = plan_mode_v2_instructions(&default_v2_inputs());
        assert!(p.contains("Launch up to 3 Explore agents IN PARALLEL"));
        assert!(p.contains("Quality over quantity - 3 agents maximum"));
        assert!(p.contains("up to 2 agent(s) in parallel"));
    }

    #[test]
    fn v2_multi_agent_block_appears_only_when_count_gt_one() {
        let single = PlanModeV2Inputs {
            plan_agent_count: 1,
            ..default_v2_inputs()
        };
        let p1 = plan_mode_v2_instructions(&single);
        assert!(!p1.contains("Multiple agents"));

        let multi = plan_mode_v2_instructions(&default_v2_inputs());
        assert!(multi.contains("Use up to 2 agents for complex tasks"));
        assert!(multi.contains("Example perspectives by task type"));
    }

    #[test]
    fn v2_plan_file_info_branches_on_plan_exists() {
        let no = plan_mode_v2_instructions(&default_v2_inputs());
        assert!(no.contains("No plan file exists yet"));
        assert!(no.contains("Write tool"));

        let yes = plan_mode_v2_instructions(&PlanModeV2Inputs {
            plan_exists: true,
            ..default_v2_inputs()
        });
        assert!(yes.contains("A plan file already exists at /plan.md"));
        assert!(yes.contains("Edit tool"));
    }

    #[test]
    fn v2_splices_phase4_section_verbatim() {
        let inputs = PlanModeV2Inputs {
            phase4_section: "### Phase 4: Final Plan\nCUSTOM_MARKER",
            ..default_v2_inputs()
        };
        let p = plan_mode_v2_instructions(&inputs);
        assert!(p.contains("CUSTOM_MARKER"));
    }

    #[test]
    fn interview_has_five_section_headings() {
        let p = plan_mode_interview_instructions(
            false,
            "/p.md",
            "Read, Grep, Glob",
            false,
        );
        for s in &[
            "### The Loop",
            "### First Turn",
            "### Asking Good Questions",
            "### Plan File Structure",
            "### When to Converge",
            "### Ending Your Turn",
        ] {
            assert!(p.contains(s), "missing {s}");
        }
    }

    #[test]
    fn interview_interpolates_read_only_tools_list() {
        let p = plan_mode_interview_instructions(
            false,
            "/p.md",
            "Read, Grep, Glob",
            false,
        );
        assert!(p.contains("Use Read, Grep, Glob to read code"));
    }

    #[test]
    fn interview_explore_sentence_gated_on_flag() {
        let off = plan_mode_interview_instructions(false, "/p.md", "Read", false);
        assert!(!off.contains("Explore agent type"));

        let on = plan_mode_interview_instructions(false, "/p.md", "Read", true);
        assert!(on.contains("You can use the Explore agent type"));
        assert!(on.contains("without filling your context"));
    }

    #[test]
    fn both_variants_reference_exit_plan_mode_tool_name() {
        let v2 = plan_mode_v2_instructions(&default_v2_inputs());
        let iv = plan_mode_interview_instructions(false, "/p.md", "Read", false);
        for p in [&v2, &iv] {
            assert!(p.contains("ExitPlanMode"));
            assert!(p.contains("AskUserQuestion"));
        }
        // And both forbid text-based plan-approval asks.
        assert!(v2.contains("Do NOT ask about plan approval"));
        assert!(iv.contains("Do NOT ask about plan approval"));
    }

    #[test]
    fn interview_plan_exists_branch_differs() {
        let no = plan_mode_interview_instructions(false, "/p.md", "Read", false);
        let yes = plan_mode_interview_instructions(true, "/p.md", "Read", false);
        assert!(no.contains("No plan file exists yet"));
        assert!(yes.contains("A plan file already exists"));
    }
}
