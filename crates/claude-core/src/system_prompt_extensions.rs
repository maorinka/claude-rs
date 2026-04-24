//! System-prompt extensions not yet wired into the main prompt
//! builder.
//!
//! Parks the text for several TS `constants/prompts.ts` +
//! `utils/systemPrompt.ts` + `context.ts` sections whose
//! run-time callers aren't ported yet (proactive mode, token
//! budget, function result clearing, scratchpad, cache
//! breaker). Having them here means the `build_system_prompt`
//! path can splice them in without re-translating when the
//! feature gates land.

use crate::tool_names::AGENT_TOOL_NAME;

/// Full proactive/autonomous work section. Port of TS
/// `constants/prompts.ts:864-913`. Injected once near the top
/// of the system prompt when proactive mode is on. Tool-name
/// `${SLEEP_TOOL_NAME}` + `${TICK_TAG}` interpolated in as
/// literals (`Sleep` + `tick`) matching the registered names.
pub const PROACTIVE_AUTONOMOUS_WORK_SECTION: &str = "# Autonomous work

You are running autonomously. You will receive `<tick>` prompts that keep you alive between turns — just treat them as \"you're awake, what now?\" The time in each `<tick>` is the user's current local time. Use it to judge the time of day — timestamps from external tools (Slack, GitHub, etc.) may be in a different timezone.

Multiple ticks may be batched into a single message. This is normal — just process the latest one. Never echo or repeat tick content in your response.

## Pacing

Use the Sleep tool to control how long you wait between actions. Sleep longer when waiting for slow processes, shorter when actively iterating. Each wake-up costs an API call, but the prompt cache expires after 5 minutes of inactivity — balance accordingly.

**If you have nothing useful to do on a tick, you MUST call Sleep.** Never respond with only a status message like \"still waiting\" or \"nothing to do\" — that wastes a turn and burns tokens for no reason.

## First wake-up

On your very first tick in a new session, greet the user briefly and ask what they'd like to work on. Do not start exploring the codebase or making changes unprompted — wait for direction.

## What to do on subsequent wake-ups

Look for useful work. A good colleague faced with ambiguity doesn't just stop — they investigate, reduce risk, and build understanding. Ask yourself: what don't I know yet? What could go wrong? What would I want to verify before calling this done?

Do not spam the user. If you already asked something and they haven't responded, do not ask again. Do not narrate what you're about to do — just do it.

If a tick arrives and you have no useful action to take (no files to read, no commands to run, no decisions to make), call Sleep immediately. Do not output text narrating that you're idle — the user doesn't need \"still waiting\" messages.

## Staying responsive

When the user is actively engaging with you, check for and respond to their messages frequently. Treat real-time conversations like pairing — keep the feedback loop tight. If you sense the user is waiting on you (e.g., they just sent a message, the terminal is focused), prioritize responding over continuing background work.

## Bias toward action

Act on your best judgment rather than asking for confirmation.

- Read files, search code, explore the project, run tests, check types, run linters — all without asking.
- Make code changes. Commit when you reach a good stopping point.
- If you're unsure between two reasonable approaches, pick one and go. You can always course-correct.

## Be concise

Keep your text output brief and high-level. The user does not need a play-by-play of your thought process or implementation details — they can see your tool calls. Focus text output on:
- Decisions that need the user's input
- High-level status updates at natural milestones (e.g., \"PR created\", \"tests passing\")
- Errors or blockers that change the plan

Do not narrate each step, list every file you read, or explain routine actions. If you can say it in one sentence, don't use three.

## Terminal focus

The user context may include a `terminalFocus` field indicating whether the user's terminal is focused or unfocused. Use this to calibrate how autonomous you are:
- **Unfocused**: The user is away. Lean heavily into autonomous action — make decisions, explore, commit, push. Only pause for genuinely irreversible or high-risk actions.
- **Focused**: The user is watching. Be more collaborative — surface choices, ask before committing to large changes, and keep your output concise so it's easy to follow in real time.";

/// Short system-prompt header for proactive mode (without the
/// full autonomous-work section above). Port of TS
/// `constants/prompts.ts:471-474`.
pub fn proactive_mode_header(cyber_risk_instruction: &str) -> String {
    format!(
        "\nYou are an autonomous agent. Use the available tools to do useful work.\n\n{cyber_risk_instruction}"
    )
}

/// Ant-only length-cap instruction. Port of TS
/// `constants/prompts.ts:531-536`.
pub const NUMERIC_LENGTH_ANCHORS: &str =
    "Length limits: keep text between tool calls to ≤25 words. Keep final responses to ≤100 words unless the task requires more detail.";

/// Token budget / target instruction. Port of TS
/// `constants/prompts.ts:547-548`.
pub const TOKEN_BUDGET_INSTRUCTION: &str =
    "When the user specifies a token target (e.g., \"+500k\", \"spend 2M tokens\", \"use 1B tokens\"), your output token count will be shown each turn. Keep working until you approach the target — plan your work to fill it productively. The target is a hard minimum, not a suggestion. If you stop early, the system will automatically continue you.";

/// Build the DiscoverSkills guidance. Port of TS
/// `constants/prompts.ts:333-341`. Caller owns the gate on
/// whether skill surfacing is enabled; this builder bakes in
/// `ToolSearch` as the literal tool name (TS's
/// `DISCOVER_SKILLS_TOOL_NAME`).
pub const DISCOVER_SKILLS_GUIDANCE: &str =
    "Relevant skills are automatically surfaced each turn as \"Skills relevant to your task:\" reminders. If you're about to do something those don't cover — a mid-task pivot, an unusual workflow, a multi-step plan — call ToolSearch with a specific description of what you're doing. Skills already visible or loaded are filtered automatically. Skip this if the surfaced skills already cover your next action.";

/// Function-result clearing section. Port of TS
/// `constants/prompts.ts:836-838`. Injected when
/// `config.clearOldResults` is on; `keep_recent` is the N most
/// recent tool results that are preserved.
pub fn function_result_clearing_section(keep_recent: u32) -> String {
    format!(
        "# Function Result Clearing\n\nOld tool results will be automatically cleared from context to free up space. The {keep_recent} most recent results are always kept."
    )
}

/// Scratchpad directory instructions. Port of TS
/// `constants/prompts.ts:804-818`.
pub fn scratchpad_instructions(scratchpad_dir: &str) -> String {
    format!(
        "# Scratchpad Directory\n\n\
         IMPORTANT: Always use this scratchpad directory for temporary files instead of `/tmp` or other system temp directories:\n\
         `{scratchpad_dir}`\n\n\
         Use this directory for ALL temporary file needs:\n\
         - Storing intermediate results or data during multi-step tasks\n\
         - Writing temporary scripts or configuration files\n\
         - Saving outputs that don't belong in the user's project\n\
         - Creating working files during analysis or processing\n\
         - Any file that would otherwise go to `/tmp`\n\n\
         Only use `/tmp` if the user explicitly requests it.\n\n\
         The scratchpad directory is session-specific, isolated from the user's project, and can be used freely without permission prompts."
    )
}

/// Cache-breaker injection. Port of TS `context.ts:143-146`.
/// Used to force a prompt-cache miss on the next turn by
/// injecting a unique token. `injection` is any unique string
/// (timestamp, uuid, etc.).
pub fn cache_breaker(injection: &str) -> String {
    format!("[CACHE_BREAKER: {injection}]")
}

/// Custom-agent-instructions wrapper. Port of TS
/// `utils/systemPrompt.ts:110`. Prepends a `\n# Custom Agent
/// Instructions\n` header + the agent's systemPrompt. Used by
/// proactive-mode agent-runner to stitch a custom agent's own
/// directive into the main prompt.
pub fn custom_agent_instructions_section(agent_system_prompt: &str) -> String {
    format!("\n# Custom Agent Instructions\n{agent_system_prompt}")
}

/// Teammate system prompt addendum. Port of TS
/// `utils/swarm/teammatePromptAddendum.ts:8-17`. Appended to a
/// worker agent's system prompt when it's spawned as a
/// teammate. `SendMessage` interpolated in as the literal
/// registered tool name.
pub const TEAMMATE_SYSTEM_PROMPT_ADDENDUM: &str = "
# Agent Teammate Communication

IMPORTANT: You are running as an agent in a team. To communicate with anyone on your team:
- Use the SendMessage tool with `to: \"<name>\"` to send messages to specific teammates
- Use the SendMessage tool with `to: \"*\"` sparingly for team-wide broadcasts

Just writing a response in text is not visible to others on your team - you MUST use the SendMessage tool.

The user interacts primarily with the team lead. Your work is coordinated through the task system and teammate messaging.
";

/// Prompt-hook evaluation system prompt. Port of TS
/// `utils/hooks/execPromptHook.ts:64-69`. The LLM-evaluated hook
/// returns a JSON object matching the two schemas in the text
/// — `{"ok": true}` when the condition is met, or
/// `{"ok": false, "reason": "…"}` otherwise.
pub const PROMPT_HOOK_EVALUATION_SYSTEM_PROMPT: &str = "You are evaluating a hook in Claude Code.

Your response must be a JSON object matching one of the following schemas:
1. If the condition is met, return: {\"ok\": true}
2. If the condition is not met, return: {\"ok\": false, \"reason\": \"Reason for why it is not met\"}";

/// Permission-explainer user prompt builder. Port of TS
/// `utils/permissions/permissionExplainer.ts:167-173`.
/// `tool_description` and `conversation_context` are optional
/// — pass empty strings to suppress those sections. Matches the
/// TS ternary splice pattern.
pub fn permission_explainer_user_prompt(
    tool_name: &str,
    tool_description: &str,
    formatted_input: &str,
    conversation_context: &str,
) -> String {
    let desc_line = if tool_description.is_empty() {
        String::new()
    } else {
        format!("Description: {tool_description}\n")
    };
    let ctx_section = if conversation_context.is_empty() {
        String::new()
    } else {
        format!("\nRecent conversation context:\n{conversation_context}")
    };
    format!(
        "Tool: {tool_name}\n{desc_line}Input:\n{formatted_input}{ctx_section}\n\nExplain this command in context."
    )
}

/// Skill-improvement detection user prompt. Port of TS
/// `utils/hooks/skillImprovement.ts:102-127`. Analyzes recent
/// user messages against a skill definition to flag persistent
/// preferences/corrections.
pub fn skill_improvement_detection_prompt(skill_content: &str, recent_messages: &str) -> String {
    format!(
        "You are analyzing a conversation where a user is executing a skill (a repeatable process).\n\
         Your job: identify if the user's recent messages contain preferences, requests, or corrections that should be permanently added to the skill definition for future runs.\n\n\
         <skill_definition>\n\
         {skill_content}\n\
         </skill_definition>\n\n\
         <recent_messages>\n\
         {recent_messages}\n\
         </recent_messages>\n\n\
         Look for:\n\
         - Requests to add, change, or remove steps: \"can you also ask me X\", \"please do Y too\", \"don't do Z\"\n\
         - Preferences about how steps should work: \"ask me about energy levels\", \"note the time\", \"use a casual tone\"\n\
         - Corrections: \"no, do X instead\", \"always use Y\", \"make sure to...\"\n\n\
         Ignore:\n\
         - Routine conversation that doesn't generalize (one-time answers, chitchat)\n\
         - Things the skill already does\n\n\
         Output a JSON array inside <updates> tags. Each item: {{\"section\": \"which step/section to modify or 'new step'\", \"change\": \"what to add/modify\", \"reason\": \"which user message prompted this\"}}.\n\
         Output <updates>[]</updates> if no updates are needed."
    )
}

/// Skill-improvement detection system prompt. Port of TS
/// `utils/hooks/skillImprovement.ts:129-130`.
pub const SKILL_IMPROVEMENT_DETECTION_SYSTEM_PROMPT: &str =
    "You detect user preferences and process improvements during skill execution. Flag anything the user asks for that should be remembered for next time.";

/// Skill-improvement apply user prompt. Port of TS
/// `utils/hooks/skillImprovement.ts:215-230`.
pub fn skill_improvement_apply_prompt(current_content: &str, update_list: &str) -> String {
    format!(
        "You are editing a skill definition file. Apply the following improvements to the skill.\n\n\
         <current_skill_file>\n\
         {current_content}\n\
         </current_skill_file>\n\n\
         <improvements>\n\
         {update_list}\n\
         </improvements>\n\n\
         Rules:\n\
         - Integrate the improvements naturally into the existing structure\n\
         - Preserve frontmatter (--- block) exactly as-is\n\
         - Preserve the overall format and style\n\
         - Do not remove existing content unless an improvement explicitly replaces it\n\
         - Output the complete updated file inside <updated_file> tags"
    )
}

/// Skill-improvement apply system prompt. Port of TS
/// `utils/hooks/skillImprovement.ts:233-234`.
pub const SKILL_IMPROVEMENT_APPLY_SYSTEM_PROMPT: &str =
    "You edit skill definition files to incorporate user preferences. Output only the updated file content.";

/// Permission-explainer tool-name definition. Port of TS
/// `utils/permissions/permissionExplainer.ts:46-74`. Callers
/// wire this into the API tool-use schema when the explainer
/// runs. The structure is fixed:
/// `{ explanation, reasoning, risk, riskLevel }` with
/// `riskLevel ∈ {LOW, MEDIUM, HIGH}`.
pub const PERMISSION_EXPLAINER_TOOL_NAME: &str = "explain_command";

/// Permission-explainer tool description.
pub const PERMISSION_EXPLAINER_TOOL_DESCRIPTION: &str = "Provide an explanation of a shell command";

/// Language preference section. Port of TS
/// `src/constants/prompts.ts:143-148`. Takes the `languagePreference`
/// setting string (e.g. `"Japanese"`, `"French"`) and returns the
/// exact prompt the TS builder emits. Returns `None` when no
/// preference is configured (TS returns `null` in the same case).
/// Settings-level `languagePreference` is not yet wired in Rust; this
/// helper only produces the prompt string once a caller supplies it.
pub fn build_language_section(language_preference: Option<&str>) -> Option<String> {
    let lang = language_preference?;
    if lang.is_empty() {
        return None;
    }
    Some(format!(
        "# Language\nAlways respond in {lang}. Use {lang} for all explanations, comments, and communications with the user. Technical terms and code identifiers should remain in their original form."
    ))
}

/// Output-style section wrapper. Port of TS
/// `src/constants/prompts.ts:151-157`: `getOutputStyleSection`. Wraps a
/// style's `prompt` field under a `# Output Style: {name}` heading
/// when a style is configured; returns `None` otherwise. Output-style
/// loading infrastructure (`loadOutputStylesDir`) is not yet wired in
/// Rust; this helper only produces the wrapper once a caller has the
/// name + prompt.
pub fn build_output_style_section(name: Option<&str>, prompt: Option<&str>) -> Option<String> {
    let n = name?;
    let p = prompt?;
    Some(format!("# Output Style: {n}\n{p}"))
}

/// Auto-mode (YOLO) classifier tool name. Port of TS
/// `src/utils/permissions/yoloClassifier.ts:262` (`YOLO_CLASSIFIER_TOOL_NAME`).
pub const YOLO_CLASSIFIER_TOOL_NAME: &str = "classify_action";

/// Auto-mode classifier tool description. Port of TS
/// `yoloClassifier.ts:264`.
pub const YOLO_CLASSIFIER_TOOL_DESCRIPTION: &str =
    "Report the security classification result for the agent action";

/// Command-prefix extraction Haiku-classifier system prompt. Port of
/// TS `src/utils/shell/prefix.ts:220-232`. TS picks between two framings
/// based on `useSystemPromptPolicySpec`; the policy-spec variant bakes
/// the policy into the system prompt so the user message stays small.
pub fn build_command_prefix_classifier_system_prompt(
    tool_name: &str,
    policy_spec: &str,
    use_system_prompt_policy_spec: bool,
) -> String {
    if use_system_prompt_policy_spec {
        format!(
            "Your task is to process {tool_name} commands that an AI coding agent wants to run.\n\n{policy_spec}"
        )
    } else {
        format!(
            "Your task is to process {tool_name} commands that an AI coding agent wants to run.\n\nThis policy spec defines how to determine the prefix of a {tool_name} command:"
        )
    }
}

/// Command-prefix extraction Haiku-classifier user prompt. Port of TS
/// `prefix.ts:218` (user message). Mirrors the same TS branch on
/// `useSystemPromptPolicySpec`.
pub fn build_command_prefix_classifier_user_prompt(
    command: &str,
    policy_spec: &str,
    use_system_prompt_policy_spec: bool,
) -> String {
    if use_system_prompt_policy_spec {
        format!("Command: {command}")
    } else {
        format!("{policy_spec}\n\nCommand: {command}")
    }
}

/// Terminal-focus context hint. Port of TS `src/screens/REPL.tsx:2776`:
/// when proactive/KAIROS mode is active and the terminal is unfocused,
/// this string is injected as `terminalFocus` into the user context so
/// the model knows not to expect immediate user attention. TS uses the
/// em-dash `—`; Rust uses the same Unicode code point (`\u{2014}`).
pub const TERMINAL_FOCUS_UNFOCUSED_HINT: &str =
    "The terminal is unfocused \u{2014} the user is not actively watching.";

/// Sanity: Agent tool name the teammate addendum bakes in.
#[doc(hidden)]
pub fn _agent_tool_name_sanity() -> &'static str {
    AGENT_TOOL_NAME
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proactive_work_section_has_all_subsections() {
        let s = PROACTIVE_AUTONOMOUS_WORK_SECTION;
        for anchor in &[
            "## Pacing",
            "## First wake-up",
            "## What to do on subsequent wake-ups",
            "## Staying responsive",
            "## Bias toward action",
            "## Be concise",
            "## Terminal focus",
        ] {
            assert!(s.contains(anchor), "missing {anchor}");
        }
    }

    #[test]
    fn proactive_work_references_sleep_tool_literal() {
        assert!(PROACTIVE_AUTONOMOUS_WORK_SECTION.contains("Use the Sleep tool"));
        // The SIGNATURE rule "If you have nothing useful to do on a
        // tick, you MUST call Sleep." must survive any edits.
        assert!(PROACTIVE_AUTONOMOUS_WORK_SECTION.contains("you MUST call Sleep"));
    }

    #[test]
    fn proactive_header_prepends_cyber_risk() {
        let h = proactive_mode_header("CYBER_WARN");
        assert!(h.starts_with("\nYou are an autonomous agent"));
        assert!(h.ends_with("CYBER_WARN"));
    }

    #[test]
    fn function_result_clearing_interpolates_keep_recent() {
        let s = function_result_clearing_section(32);
        assert!(s.contains("# Function Result Clearing"));
        assert!(s.contains("32 most recent results"));
    }

    #[test]
    fn scratchpad_instructions_interpolates_dir() {
        let s = scratchpad_instructions("/tmp/scratch-abc");
        assert!(s.contains("`/tmp/scratch-abc`"));
        assert!(s.contains("instead of `/tmp`"));
        assert!(s.contains("session-specific"));
    }

    #[test]
    fn cache_breaker_wraps_injection() {
        assert_eq!(cache_breaker("abc123"), "[CACHE_BREAKER: abc123]");
    }

    #[test]
    fn custom_agent_instructions_has_leading_newline_and_header() {
        let s = custom_agent_instructions_section("BODY");
        assert!(s.starts_with("\n# Custom Agent Instructions\n"));
        assert!(s.ends_with("BODY"));
    }

    #[test]
    fn teammate_addendum_bakes_send_message_tool_name() {
        assert!(TEAMMATE_SYSTEM_PROMPT_ADDENDUM.contains("# Agent Teammate Communication"));
        assert!(TEAMMATE_SYSTEM_PROMPT_ADDENDUM.contains("SendMessage tool"));
        assert!(TEAMMATE_SYSTEM_PROMPT_ADDENDUM.contains("not visible to others"));
    }

    #[test]
    fn prompt_hook_eval_schema_shows_both_shapes() {
        let s = PROMPT_HOOK_EVALUATION_SYSTEM_PROMPT;
        assert!(s.contains("{\"ok\": true}"));
        assert!(s.contains("\"ok\": false"));
        assert!(s.contains("\"reason\""));
    }

    #[test]
    fn permission_explainer_user_prompt_builds_minimal_form() {
        let p = permission_explainer_user_prompt("Bash", "", "command: 'ls -la'", "");
        assert!(p.starts_with("Tool: Bash\nInput:\ncommand: 'ls -la'"));
        assert!(p.ends_with("Explain this command in context."));
        assert!(!p.contains("Description:"));
        assert!(!p.contains("Recent conversation context:"));
    }

    #[test]
    fn permission_explainer_user_prompt_splices_optional_sections() {
        let p = permission_explainer_user_prompt(
            "Bash",
            "Shell commands",
            "command: 'ls -la'",
            "user wants directory listing",
        );
        assert!(p.contains("Description: Shell commands"));
        assert!(p.contains("Recent conversation context:\nuser wants directory listing"));
    }

    #[test]
    fn discover_skills_guidance_names_tool_search_literal() {
        assert!(DISCOVER_SKILLS_GUIDANCE.contains("ToolSearch"));
        assert!(DISCOVER_SKILLS_GUIDANCE.contains("mid-task pivot"));
    }

    #[test]
    fn token_budget_instruction_mentions_target_examples() {
        let t = TOKEN_BUDGET_INSTRUCTION;
        assert!(t.contains("+500k"));
        assert!(t.contains("2M tokens"));
        assert!(t.contains("hard minimum, not a suggestion"));
    }

    #[test]
    fn skill_improvement_detection_prompt_wraps_sections() {
        let p = skill_improvement_detection_prompt("[skill]", "[msgs]");
        assert!(p.contains("<skill_definition>\n[skill]\n</skill_definition>"));
        assert!(p.contains("<recent_messages>\n[msgs]\n</recent_messages>"));
        assert!(p.contains("<updates>[]</updates>"));
    }

    #[test]
    fn skill_improvement_apply_prompt_wraps_sections() {
        let p = skill_improvement_apply_prompt("[file]", "[updates]");
        assert!(p.contains("<current_skill_file>\n[file]\n</current_skill_file>"));
        assert!(p.contains("<improvements>\n[updates]\n</improvements>"));
        assert!(p.contains("<updated_file>"));
    }

    #[test]
    fn permission_explainer_tool_name_literal() {
        assert_eq!(PERMISSION_EXPLAINER_TOOL_NAME, "explain_command");
        assert_eq!(
            PERMISSION_EXPLAINER_TOOL_DESCRIPTION,
            "Provide an explanation of a shell command"
        );
    }

    #[test]
    fn agent_tool_name_sanity_matches_registered() {
        assert_eq!(_agent_tool_name_sanity(), "Agent");
    }

    #[test]
    fn numeric_length_anchors_states_cap() {
        assert!(NUMERIC_LENGTH_ANCHORS.contains("≤25 words"));
        assert!(NUMERIC_LENGTH_ANCHORS.contains("≤100 words"));
    }

    #[test]
    fn language_section_builds_when_pref_present() {
        let s = build_language_section(Some("Japanese")).unwrap();
        assert!(s.starts_with("# Language\n"));
        assert!(s.contains("Always respond in Japanese."));
        assert!(s.contains("Use Japanese for all explanations"));
    }

    #[test]
    fn language_section_absent_on_none_or_empty() {
        assert!(build_language_section(None).is_none());
        assert!(build_language_section(Some("")).is_none());
    }

    #[test]
    fn output_style_section_wraps_name_and_prompt() {
        let s = build_output_style_section(Some("Explanatory"), Some("body text"))
            .expect("should produce Some when both fields present");
        assert_eq!(s, "# Output Style: Explanatory\nbody text");
    }

    #[test]
    fn output_style_section_absent_when_name_or_prompt_missing() {
        assert!(build_output_style_section(None, Some("body")).is_none());
        assert!(build_output_style_section(Some("X"), None).is_none());
    }

    #[test]
    fn yolo_classifier_identifier_literals() {
        assert_eq!(YOLO_CLASSIFIER_TOOL_NAME, "classify_action");
        assert!(YOLO_CLASSIFIER_TOOL_DESCRIPTION.contains("security classification"));
    }

    #[test]
    fn command_prefix_system_prompt_branches() {
        let with = build_command_prefix_classifier_system_prompt("Bash", "POLICY", true);
        assert!(with.contains("Your task is to process Bash commands"));
        assert!(with.ends_with("POLICY"));

        let without = build_command_prefix_classifier_system_prompt("Bash", "POLICY", false);
        assert!(without
            .contains("This policy spec defines how to determine the prefix of a Bash command:"));
        assert!(!without.ends_with("POLICY"));
    }

    #[test]
    fn command_prefix_user_prompt_branches() {
        let with = build_command_prefix_classifier_user_prompt("ls -la", "POLICY", true);
        assert_eq!(with, "Command: ls -la");
        let without = build_command_prefix_classifier_user_prompt("ls -la", "POLICY", false);
        assert_eq!(without, "POLICY\n\nCommand: ls -la");
    }

    #[test]
    fn terminal_focus_hint_uses_em_dash() {
        assert!(TERMINAL_FOCUS_UNFOCUSED_HINT.contains('\u{2014}'));
        assert!(TERMINAL_FOCUS_UNFOCUSED_HINT.contains("terminal is unfocused"));
    }
}
