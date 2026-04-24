//! `/insights` multi-step pipeline prompts.
//!
//! Port of TS `src/commands/insights.ts` — the multi-step
//! pipeline that chunks recent transcripts, extracts structured
//! facets per chunk, runs per-section analyses in parallel, and
//! synthesizes an "At a Glance" summary for the final HTML
//! report. The Rust `/insights` command today uses a simplified
//! single-prompt path (builtin.rs:2188); none of these pipeline
//! prompts are consumed yet. Parking them here keeps the text
//! in the binary + diff-stable against TS for when the pipeline
//! lands.
//!
//! # Scope
//!
//! - `SUMMARIZE_CHUNK_PROMPT` — per-chunk summarizer.
//! - `FACET_EXTRACTION_PROMPT` + [`facet_extraction_json_prompt`]
//!   — facet extractor + the JSON-schema-appended variant.
//! - `INSIGHT_SECTION_*` constants — one per pipeline section.
//! - `AT_A_GLANCE_PROMPT_TEMPLATE` + [`at_a_glance_prompt`] —
//!   the synthesis prompt with seven dynamic slots.
//! - [`insights_final_report_message`] — the final user-visible
//!   message that includes the report URL + shareable snippet.

/// Per-chunk summarizer prompt. Port of TS
/// `SUMMARIZE_CHUNK_PROMPT` at insights.ts:870-878.
pub const SUMMARIZE_CHUNK_PROMPT: &str = "Summarize this portion of a Claude Code session transcript. Focus on:
1. What the user asked for
2. What Claude did (tools used, files modified)
3. Any friction or issues
4. The outcome

Keep it concise - 3-5 sentences. Preserve specific details like file names, error messages, and user feedback.

TRANSCRIPT CHUNK:
";

/// Facet-extractor prompt (no JSON schema appended). Port of TS
/// `FACET_EXTRACTION_PROMPT` at insights.ts:430-456.
pub const FACET_EXTRACTION_PROMPT: &str =
    "Analyze this Claude Code session and extract structured facets.

CRITICAL GUIDELINES:

1. **goal_categories**: Count ONLY what the USER explicitly asked for.
   - DO NOT count Claude's autonomous codebase exploration
   - DO NOT count work Claude decided to do on its own
   - ONLY count when user says \"can you...\", \"please...\", \"I need...\", \"let's...\"

2. **user_satisfaction_counts**: Base ONLY on explicit user signals.
   - \"Yay!\", \"great!\", \"perfect!\" -> happy
   - \"thanks\", \"looks good\", \"that works\" -> satisfied
   - \"ok, now let's...\" (continuing without complaint) -> likely_satisfied
   - \"that's not right\", \"try again\" -> dissatisfied
   - \"this is broken\", \"I give up\" -> frustrated

3. **friction_counts**: Be specific about what went wrong.
   - misunderstood_request: Claude interpreted incorrectly
   - wrong_approach: Right goal, wrong solution method
   - buggy_code: Code didn't work correctly
   - user_rejected_action: User said no/stop to a tool call
   - excessive_changes: Over-engineered or changed too much

4. If very short or just warmup, use warmup_minimal for goal_category

SESSION:
";

/// JSON-response suffix appended after the transcript. Port of
/// TS `insights.ts:1010-1024`.
pub const FACET_EXTRACTION_JSON_SUFFIX: &str = "\n\nRESPOND WITH ONLY A VALID JSON OBJECT matching this schema:
{
  \"underlying_goal\": \"What the user fundamentally wanted to achieve\",
  \"goal_categories\": {\"category_name\": count, ...},
  \"outcome\": \"fully_achieved|mostly_achieved|partially_achieved|not_achieved|unclear_from_transcript\",
  \"user_satisfaction_counts\": {\"level\": count, ...},
  \"claude_helpfulness\": \"unhelpful|slightly_helpful|moderately_helpful|very_helpful|essential\",
  \"session_type\": \"single_task|multi_task|iterative_refinement|exploration|quick_question\",
  \"friction_counts\": {\"friction_type\": count, ...},
  \"friction_detail\": \"One sentence describing friction or empty\",
  \"primary_success\": \"none|fast_accurate_search|correct_code_edits|good_explanations|proactive_help|multi_file_changes|good_debugging\",
  \"brief_summary\": \"One sentence: what user wanted and whether they got it\"
}";

/// Build the JSON-schema-augmented facet-extraction prompt.
/// Appends `transcript` after the guidelines then the JSON
/// schema after that — matches TS concatenation order at
/// `insights.ts:1010-1024`.
pub fn facet_extraction_json_prompt(transcript: &str) -> String {
    format!("{FACET_EXTRACTION_PROMPT}{transcript}{FACET_EXTRACTION_JSON_SUFFIX}")
}

/// INSIGHT_SECTIONS entry: `project_areas`. Port of
/// insights.ts:1337-1348.
pub const INSIGHT_SECTION_PROJECT_AREAS: &str = "Analyze this Claude Code usage data and identify project areas.

RESPOND WITH ONLY A VALID JSON OBJECT:
{
  \"areas\": [
    {\"name\": \"Area name\", \"session_count\": N, \"description\": \"2-3 sentences about what was worked on and how Claude Code was used.\"}
  ]
}

Include 4-5 areas. Skip internal CC operations.";

/// INSIGHT_SECTIONS entry: `interaction_style`. Port of
/// insights.ts:1351-1360.
pub const INSIGHT_SECTION_INTERACTION_STYLE: &str = "Analyze this Claude Code usage data and describe the user's interaction style.

RESPOND WITH ONLY A VALID JSON OBJECT:
{
  \"narrative\": \"2-3 paragraphs analyzing HOW the user interacts with Claude Code. Use second person 'you'. Describe patterns: iterate quickly vs detailed upfront specs? Interrupt often or let Claude run? Include specific examples. Use **bold** for key insights.\",
  \"key_pattern\": \"One sentence summary of most distinctive interaction style\"
}";

/// INSIGHT_SECTIONS entry: `what_works`. Port of
/// insights.ts:1362-1375.
pub const INSIGHT_SECTION_WHAT_WORKS: &str = "Analyze this Claude Code usage data and identify what's working well for this user. Use second person (\"you\").

RESPOND WITH ONLY A VALID JSON OBJECT:
{
  \"intro\": \"1 sentence of context\",
  \"impressive_workflows\": [
    {\"title\": \"Short title (3-6 words)\", \"description\": \"2-3 sentences describing the impressive workflow or approach. Use 'you' not 'the user'.\"}
  ]
}

Include 3 impressive workflows.";

/// INSIGHT_SECTIONS entry: `friction_analysis`. Port of
/// insights.ts:1377-1390.
pub const INSIGHT_SECTION_FRICTION_ANALYSIS: &str = "Analyze this Claude Code usage data and identify friction points for this user. Use second person (\"you\").

RESPOND WITH ONLY A VALID JSON OBJECT:
{
  \"intro\": \"1 sentence summarizing friction patterns\",
  \"categories\": [
    {\"category\": \"Concrete category name\", \"description\": \"1-2 sentences explaining this category and what could be done differently. Use 'you' not 'the user'.\", \"examples\": [\"Specific example with consequence\", \"Another example\"]}
  ]
}

Include 3 friction categories with 2 examples each.";

/// INSIGHT_SECTIONS entry: `suggestions`. Port of
/// insights.ts:1392-1433 — includes the embedded "CC FEATURES
/// REFERENCE" block.
pub const INSIGHT_SECTION_SUGGESTIONS: &str = "Analyze this Claude Code usage data and suggest improvements.

## CC FEATURES REFERENCE (pick from these for features_to_try):
1. **MCP Servers**: Connect Claude to external tools, databases, and APIs via Model Context Protocol.
   - How to use: Run `claude mcp add <server-name> -- <command>`
   - Good for: database queries, Slack integration, GitHub issue lookup, connecting to internal APIs

2. **Custom Skills**: Reusable prompts you define as markdown files that run with a single /command.
   - How to use: Create `.claude/skills/commit/SKILL.md` with instructions. Then type `/commit` to run it.
   - Good for: repetitive workflows - /commit, /review, /test, /deploy, /pr, or complex multi-step workflows

3. **Hooks**: Shell commands that auto-run at specific lifecycle events.
   - How to use: Add to `.claude/settings.json` under \"hooks\" key.
   - Good for: auto-formatting code, running type checks, enforcing conventions

4. **Headless Mode**: Run Claude non-interactively from scripts and CI/CD.
   - How to use: `claude -p \"fix lint errors\" --allowedTools \"Edit,Read,Bash\"`
   - Good for: CI/CD integration, batch code fixes, automated reviews

5. **Task Agents**: Claude spawns focused sub-agents for complex exploration or parallel work.
   - How to use: Claude auto-invokes when helpful, or ask \"use an agent to explore X\"
   - Good for: codebase exploration, understanding complex systems

RESPOND WITH ONLY A VALID JSON OBJECT:
{
  \"claude_md_additions\": [
    {\"addition\": \"A specific line or block to add to CLAUDE.md based on workflow patterns...\", \"why\": \"1 sentence...\", \"prompt_scaffold\": \"Instructions for where to add this in CLAUDE.md...\"}
  ],
  \"features_to_try\": [
    {\"feature\": \"Feature name from CC FEATURES REFERENCE above\", \"one_liner\": \"What it does\", \"why_for_you\": \"Why this would help YOU based on your sessions\", \"example_code\": \"Actual command or config to copy\"}
  ],
  \"usage_patterns\": [
    {\"title\": \"Short title\", \"suggestion\": \"1-2 sentence summary\", \"detail\": \"3-4 sentences explaining how this applies to YOUR work\", \"copyable_prompt\": \"A specific prompt to copy and try\"}
  ]
}

IMPORTANT for claude_md_additions: PRIORITIZE instructions that appear MULTIPLE TIMES in the user data. If user told Claude the same thing in 2+ sessions (e.g., 'always run tests', 'use TypeScript'), that's a PRIME candidate - they shouldn't have to repeat themselves.

IMPORTANT for features_to_try: Pick 2-3 from the CC FEATURES REFERENCE above. Include 2-3 items for each category.";

/// INSIGHT_SECTIONS entry: `on_the_horizon`. Port of
/// insights.ts:1435-1448.
pub const INSIGHT_SECTION_ON_THE_HORIZON: &str = "Analyze this Claude Code usage data and identify future opportunities.

RESPOND WITH ONLY A VALID JSON OBJECT:
{
  \"intro\": \"1 sentence about evolving AI-assisted development\",
  \"opportunities\": [
    {\"title\": \"Short title (4-8 words)\", \"whats_possible\": \"2-3 ambitious sentences about autonomous workflows\", \"how_to_try\": \"1-2 sentences mentioning relevant tooling\", \"copyable_prompt\": \"Detailed prompt to try\"}
  ]
}

Include 3 opportunities. Think BIG - autonomous workflows, parallel agents, iterating against tests.";

/// INSIGHT_SECTIONS entry: `cc_team_improvements` (ant-only).
/// Port of insights.ts:1452-1465.
pub const INSIGHT_SECTION_CC_TEAM_IMPROVEMENTS: &str = "Analyze this Claude Code usage data and suggest product improvements for the CC team.

RESPOND WITH ONLY A VALID JSON OBJECT:
{
  \"improvements\": [
    {\"title\": \"Product/tooling improvement\", \"detail\": \"3-4 sentences describing the improvement\", \"evidence\": \"3-4 sentences with specific session examples\"}
  ]
}

Include 2-3 improvements based on friction patterns observed.";

/// INSIGHT_SECTIONS entry: `model_behavior_improvements`
/// (ant-only). Port of insights.ts:1467-1479.
pub const INSIGHT_SECTION_MODEL_BEHAVIOR_IMPROVEMENTS: &str = "Analyze this Claude Code usage data and suggest model behavior improvements.

RESPOND WITH ONLY A VALID JSON OBJECT:
{
  \"improvements\": [
    {\"title\": \"Model behavior change\", \"detail\": \"3-4 sentences describing what the model should do differently\", \"evidence\": \"3-4 sentences with specific examples\"}
  ]
}

Include 2-3 improvements based on friction patterns observed.";

/// INSIGHT_SECTIONS entry: `fun_ending`. Port of
/// insights.ts:1482-1494.
pub const INSIGHT_SECTION_FUN_ENDING: &str = "Analyze this Claude Code usage data and find a memorable moment.

RESPOND WITH ONLY A VALID JSON OBJECT:
{
  \"headline\": \"A memorable QUALITATIVE moment from the transcripts - not a statistic. Something human, funny, or surprising.\",
  \"detail\": \"Brief context about when/where this happened\"
}

Find something genuinely interesting or amusing from the session summaries.";

/// Template for the `At a Glance` synthesis prompt, with seven
/// `{{SLOT}}` placeholders the caller fills via [`at_a_glance_prompt`].
/// Port of TS `insights.ts:1738-1779` `atAGlancePrompt`.
const AT_A_GLANCE_TEMPLATE: &str = "You're writing an \"At a Glance\" summary for a Claude Code usage insights report for Claude Code users. The goal is to help them understand their usage and improve how they can use Claude better, especially as models improve.

Use this 4-part structure:

1. **What's working** - What is the user's unique style of interacting with Claude and what are some impactful things they've done? You can include one or two details, but keep it high level since things might not be fresh in the user's memory. Don't be fluffy or overly complimentary. Also, don't focus on the tool calls they use.

2. **What's hindering you** - Split into (a) Claude's fault (misunderstandings, wrong approaches, bugs) and (b) user-side friction (not providing enough context, environment issues -- ideally more general than just one project). Be honest but constructive.

3. **Quick wins to try** - Specific Claude Code features they could try from the examples below, or a workflow technique if you think it's really compelling. (Avoid stuff like \"Ask Claude to confirm before taking actions\" or \"Type out more context up front\" which are less compelling.)

4. **Ambitious workflows for better models** - As we move to much more capable models over the next 3-6 months, what should they prepare for? What workflows that seem impossible now will become possible? Draw from the appropriate section below.

Keep each section to 2-3 not-too-long sentences. Don't overwhelm the user. Don't mention specific numerical stats or underlined_categories from the session data below. Use a coaching tone.

RESPOND WITH ONLY A VALID JSON OBJECT:
{
  \"whats_working\": \"(refer to instructions above)\",
  \"whats_hindering\": \"(refer to instructions above)\",
  \"quick_wins\": \"(refer to instructions above)\",
  \"ambitious_workflows\": \"(refer to instructions above)\"
}

SESSION DATA:
{{FULL_CONTEXT}}

## Project Areas (what user works on)
{{PROJECT_AREAS}}

## Big Wins (impressive accomplishments)
{{BIG_WINS}}

## Friction Categories (where things go wrong)
{{FRICTION}}

## Features to Try
{{FEATURES}}

## Usage Patterns to Adopt
{{PATTERNS}}

## On the Horizon (ambitious workflows for better models)
{{HORIZON}}";

/// Inputs for [`at_a_glance_prompt`].
pub struct AtAGlanceInputs<'a> {
    pub full_context: &'a str,
    pub project_areas_text: &'a str,
    pub big_wins_text: &'a str,
    pub friction_text: &'a str,
    pub features_text: &'a str,
    pub patterns_text: &'a str,
    pub horizon_text: &'a str,
}

/// Fill the seven dynamic slots in [`AT_A_GLANCE_TEMPLATE`]. All
/// inputs are pre-rendered text blocks the caller assembles from
/// the per-section pipeline outputs.
pub fn at_a_glance_prompt(inputs: &AtAGlanceInputs<'_>) -> String {
    AT_A_GLANCE_TEMPLATE
        .replace("{{FULL_CONTEXT}}", inputs.full_context)
        .replace("{{PROJECT_AREAS}}", inputs.project_areas_text)
        .replace("{{BIG_WINS}}", inputs.big_wins_text)
        .replace("{{FRICTION}}", inputs.friction_text)
        .replace("{{FEATURES}}", inputs.features_text)
        .replace("{{PATTERNS}}", inputs.patterns_text)
        .replace("{{HORIZON}}", inputs.horizon_text)
}

/// Build the user-visible message the TS command returns after
/// the pipeline finishes. Port of TS `insights.ts:3156-3180`.
///
/// - `insights_json` is `JSON.stringify(insights, null, 2)` of
///   the full pipeline output.
/// - `report_url` is the shareable URL.
/// - `html_path` is the local HTML report file.
/// - `facets_dir` is the directory holding per-facet JSON.
/// - `user_summary` is the pretty-printed version the user sees.
/// - `upload_hint` optionally extends the share line (TS uses an
///   empty string when uploads aren't configured).
pub fn insights_final_report_message(
    insights_json: &str,
    report_url: &str,
    html_path: &str,
    facets_dir: &str,
    user_summary: &str,
    upload_hint: &str,
) -> String {
    format!(
        "The user just ran /insights to generate a usage report analyzing their Claude Code sessions.\n\n\
         Here is the full insights data:\n\
         {insights_json}\n\n\
         Report URL: {report_url}\n\
         HTML file: {html_path}\n\
         Facets directory: {facets_dir}\n\n\
         Here is what the user sees:\n\
         {user_summary}\n\n\
         Now output the following message exactly:\n\n\
         <message>\n\
         Your shareable insights report is ready:\n\
         {report_url}{upload_hint}\n\n\
         Want to dig into any section or try one of the suggestions?\n\
         </message>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_chunk_prompt_is_concise_and_ends_on_transcript_anchor() {
        assert!(SUMMARIZE_CHUNK_PROMPT.starts_with("Summarize this portion"));
        assert!(SUMMARIZE_CHUNK_PROMPT.ends_with("TRANSCRIPT CHUNK:\n"));
    }

    #[test]
    fn facet_extraction_has_four_critical_guidelines() {
        for anchor in &[
            "1. **goal_categories**",
            "2. **user_satisfaction_counts**",
            "3. **friction_counts**",
            "4. If very short or just warmup",
        ] {
            assert!(
                FACET_EXTRACTION_PROMPT.contains(anchor),
                "facet extraction missing `{anchor}`"
            );
        }
    }

    #[test]
    fn facet_extraction_json_prompt_concatenates_three_pieces() {
        let p = facet_extraction_json_prompt("<T>");
        assert!(p.contains("CRITICAL GUIDELINES"));
        assert!(p.contains("<T>"));
        assert!(p.contains("RESPOND WITH ONLY A VALID JSON OBJECT"));
        assert!(p.contains("\"underlying_goal\""));
    }

    #[test]
    fn every_insight_section_demands_json_output() {
        for (name, body) in [
            ("project_areas", INSIGHT_SECTION_PROJECT_AREAS),
            ("interaction_style", INSIGHT_SECTION_INTERACTION_STYLE),
            ("what_works", INSIGHT_SECTION_WHAT_WORKS),
            ("friction_analysis", INSIGHT_SECTION_FRICTION_ANALYSIS),
            ("suggestions", INSIGHT_SECTION_SUGGESTIONS),
            ("on_the_horizon", INSIGHT_SECTION_ON_THE_HORIZON),
            ("cc_team_improvements", INSIGHT_SECTION_CC_TEAM_IMPROVEMENTS),
            (
                "model_behavior_improvements",
                INSIGHT_SECTION_MODEL_BEHAVIOR_IMPROVEMENTS,
            ),
            ("fun_ending", INSIGHT_SECTION_FUN_ENDING),
        ] {
            assert!(
                body.contains("RESPOND WITH ONLY A VALID JSON OBJECT"),
                "section `{name}` missing JSON rule"
            );
        }
    }

    #[test]
    fn suggestions_references_five_cc_features() {
        let s = INSIGHT_SECTION_SUGGESTIONS;
        assert!(s.contains("1. **MCP Servers**"));
        assert!(s.contains("2. **Custom Skills**"));
        assert!(s.contains("3. **Hooks**"));
        assert!(s.contains("4. **Headless Mode**"));
        assert!(s.contains("5. **Task Agents**"));
    }

    #[test]
    fn at_a_glance_keeps_four_part_structure_and_fills_slots() {
        let inputs = AtAGlanceInputs {
            full_context: "<CTX>",
            project_areas_text: "<PA>",
            big_wins_text: "<BW>",
            friction_text: "<FR>",
            features_text: "<FE>",
            patterns_text: "<PT>",
            horizon_text: "<HO>",
        };
        let p = at_a_glance_prompt(&inputs);
        // 4-part structure headings still present.
        for anchor in &[
            "1. **What's working**",
            "2. **What's hindering you**",
            "3. **Quick wins to try**",
            "4. **Ambitious workflows for better models**",
        ] {
            assert!(p.contains(anchor));
        }
        for marker in &["<CTX>", "<PA>", "<BW>", "<FR>", "<FE>", "<PT>", "<HO>"] {
            assert!(p.contains(marker), "slot `{marker}` not filled");
        }
        // No unsubstituted slots.
        for slot in &[
            "{{FULL_CONTEXT}}",
            "{{PROJECT_AREAS}}",
            "{{BIG_WINS}}",
            "{{FRICTION}}",
            "{{FEATURES}}",
            "{{PATTERNS}}",
            "{{HORIZON}}",
        ] {
            assert!(!p.contains(slot), "slot `{slot}` left over");
        }
    }

    #[test]
    fn final_report_message_embeds_url_html_facets_and_shareable_block() {
        let msg = insights_final_report_message(
            "{\"x\":1}",
            "https://claude.ai/code/insights/abc",
            "/tmp/report.html",
            "/tmp/facets",
            "(summary text)",
            "", // no upload hint
        );
        assert!(msg.contains("Report URL: https://claude.ai/code/insights/abc"));
        assert!(msg.contains("HTML file: /tmp/report.html"));
        assert!(msg.contains("Facets directory: /tmp/facets"));
        assert!(msg.contains("<message>"));
        assert!(msg.contains("</message>"));
        assert!(msg.contains("Your shareable insights report is ready:"));
        assert!(msg.contains("Want to dig into any section"));
    }

    #[test]
    fn final_report_message_appends_upload_hint_after_url() {
        let msg = insights_final_report_message(
            "{}",
            "https://example.com/r",
            "/tmp/r.html",
            "/tmp/f",
            "",
            " (upload in progress)",
        );
        // The hint immediately follows the URL inside the
        // `<message>` block — TS interpolates `${reportUrl}${uploadHint}`.
        assert!(msg.contains("https://example.com/r (upload in progress)"));
    }
}
