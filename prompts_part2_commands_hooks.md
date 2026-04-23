# Part 2: Commands & Hooks

## [commit.ts]
### /commit command prompt - Git commit creation
**File:** `src/commands/commit.ts:20-54`
```ts
function getPromptContent(): string {
  const { commit: commitAttribution } = getAttributionTexts()

  let prefix = ''
  if (process.env.USER_TYPE === 'ant' && isUndercover()) {
    prefix = getUndercoverInstructions() + '\n'
  }

  return `${prefix}## Context

- Current git status: !\`git status\`
- Current git diff (staged and unstaged changes): !\`git diff HEAD\`
- Current branch: !\`git branch --show-current\`
- Recent commits: !\`git log --oneline -10\`

## Git Safety Protocol

- NEVER update the git config
- NEVER skip hooks (--no-verify, --no-gpg-sign, etc) unless the user explicitly requests it
- CRITICAL: ALWAYS create NEW commits. NEVER use git commit --amend, unless the user explicitly requests it
- Do not commit files that likely contain secrets (.env, credentials.json, etc). Warn the user if they specifically request to commit those files
- If there are no changes to commit (i.e., no untracked files and no modifications), do not create an empty commit
- Never use git commands with the -i flag (like git rebase -i or git add -i) since they require interactive input which is not supported

## Your task

Based on the above changes, create a single git commit:

1. Analyze all staged changes and draft a commit message:
   - Look at the recent commits above to follow this repository's commit message style
   - Summarize the nature of the changes (new feature, enhancement, bug fix, refactoring, test, docs, etc.)
   - Ensure the message accurately reflects the changes and their purpose (i.e. "add" means a wholly new feature, "update" means an enhancement to an existing feature, "fix" means a bug fix, etc.)
   - Draft a concise (1-2 sentences) commit message that focuses on the "why" rather than the "what"

2. Stage relevant files and create the commit using HEREDOC syntax:
\`\`\`
git commit -m "$(cat <<'EOF'
Commit message here.${commitAttribution ? `\n\n${commitAttribution}` : ''}
EOF
)"
\`\`\`

You have the capability to call multiple tools in a single response. Stage and create the commit using a single message. Do not use any other tools or do anything else. Do not send any other text or messages besides these tool calls.`
}
```
**Status: ADDED to Rust** -- `crates/claude-core/src/commands/builtin.rs:1145` (CommitHandler updated with full TS prompt including Git Safety Protocol, context gathering, HEREDOC syntax, and commit message guidelines. Note: commitAttribution / undercover prefix not ported as those are Anthropic-internal features.)

---

## [commit-push-pr.ts]
### /commit-push-pr command prompt - Full commit, push, and PR creation
**File:** `src/commands/commit-push-pr.ts:26-106`
```ts
function getPromptContent(
  defaultBranch: string,
  prAttribution?: string,
): string {
  const { commit: commitAttribution, pr: defaultPrAttribution } =
    getAttributionTexts()
  const effectivePrAttribution = prAttribution ?? defaultPrAttribution
  const safeUser = process.env.SAFEUSER || ''
  const username = process.env.USER || ''

  let prefix = ''
  let reviewerArg = ' and `--reviewer anthropics/claude-code`'
  let addReviewerArg = ' (and add `--add-reviewer anthropics/claude-code`)'
  let changelogSection = `

## Changelog
<!-- CHANGELOG:START -->
[If this PR contains user-facing changes, add a changelog entry here. Otherwise, remove this section.]
<!-- CHANGELOG:END -->`
  let slackStep = `

5. After creating/updating the PR, check if the user's CLAUDE.md mentions posting to Slack channels. If it does, use ToolSearch to search for "slack send message" tools. If ToolSearch finds a Slack tool, ask the user if they'd like you to post the PR URL to the relevant Slack channel. Only post if the user confirms. If ToolSearch returns no results or errors, skip this step silently—do not mention the failure, do not attempt workarounds, and do not try alternative approaches.`
  if (process.env.USER_TYPE === 'ant' && isUndercover()) {
    prefix = getUndercoverInstructions() + '\n'
    reviewerArg = ''
    addReviewerArg = ''
    changelogSection = ''
    slackStep = ''
  }

  return `${prefix}## Context

- \`SAFEUSER\`: ${safeUser}
- \`whoami\`: ${username}
- \`git status\`: !\`git status\`
- \`git diff HEAD\`: !\`git diff HEAD\`
- \`git branch --show-current\`: !\`git branch --show-current\`
- \`git diff ${defaultBranch}...HEAD\`: !\`git diff ${defaultBranch}...HEAD\`
- \`gh pr view --json number 2>/dev/null || true\`: !\`gh pr view --json number 2>/dev/null || true\`

## Git Safety Protocol

- NEVER update the git config
- NEVER run destructive/irreversible git commands (like push --force, hard reset, etc) unless the user explicitly requests them
- NEVER skip hooks (--no-verify, --no-gpg-sign, etc) unless the user explicitly requests it
- NEVER run force push to main/master, warn the user if they request it
- Do not commit files that likely contain secrets (.env, credentials.json, etc)
- Never use git commands with the -i flag (like git rebase -i or git add -i) since they require interactive input which is not supported

## Your task

Analyze all changes that will be included in the pull request, making sure to look at all relevant commits (NOT just the latest commit, but ALL commits that will be included in the pull request from the git diff ${defaultBranch}...HEAD output above).

Based on the above changes:
1. Create a new branch if on ${defaultBranch} (use SAFEUSER from context above for the branch name prefix, falling back to whoami if SAFEUSER is empty, e.g., \`username/feature-name\`)
2. Create a single commit with an appropriate message using heredoc syntax${commitAttribution ? `, ending with the attribution text shown in the example below` : ''}:
\`\`\`
git commit -m "$(cat <<'EOF'
Commit message here.${commitAttribution ? `\n\n${commitAttribution}` : ''}
EOF
)"
\`\`\`
3. Push the branch to origin
4. If a PR already exists for this branch (check the gh pr view output above), update the PR title and body using \`gh pr edit\` to reflect the current diff${addReviewerArg}. Otherwise, create a pull request using \`gh pr create\` with heredoc syntax for the body${reviewerArg}.
   - IMPORTANT: Keep PR titles short (under 70 characters). Use the body for details.
\`\`\`
gh pr create --title "Short, descriptive title" --body "$(cat <<'EOF'
## Summary
<1-3 bullet points>

## Test plan
[Bulleted markdown checklist of TODOs for testing the pull request...]${changelogSection}${effectivePrAttribution ? `\n\n${effectivePrAttribution}` : ''}
EOF
)"
\`\`\`

You have the capability to call multiple tools in a single response. You MUST do all of the above in a single message.${slackStep}

Return the PR URL when you're done, so the user can see it.`
}
```
**Status: ADDED to Rust** -- `crates/claude-core/src/commands/builtin.rs:2001` (CommitPushPrHandler updated with full TS prompt including SAFEUSER/whoami context, Git Safety Protocol, default branch detection, HEREDOC syntax for commit and PR body, `gh pr view` / `gh pr edit` / `gh pr create` flow. Note: commitAttribution, prAttribution, undercover prefix, reviewerArg, changelogSection, and slackStep not ported as those are Anthropic-internal features.)

---

## [review.ts]
### /review command prompt - Local PR code review
**File:** `src/commands/review.ts:9-31`
```ts
const LOCAL_REVIEW_PROMPT = (args: string) => `
      You are an expert code reviewer. Follow these steps:

      1. If no PR number is provided in the args, run \`gh pr list\` to show open PRs
      2. If a PR number is provided, run \`gh pr view <number>\` to get PR details
      3. Run \`gh pr diff <number>\` to get the diff
      4. Analyze the changes and provide a thorough code review that includes:
         - Overview of what the PR does
         - Analysis of code quality and style
         - Specific suggestions for improvements
         - Any potential issues or risks

      Keep your review concise but thorough. Focus on:
      - Code correctness
      - Following project conventions
      - Performance implications
      - Test coverage
      - Security considerations

      Format your review with clear sections and bullet points.

      PR number: ${args}
    `
```
**Status: ADDED to Rust** -- `crates/claude-core/src/commands/builtin.rs:1181` (ReviewHandler updated with full TS prompt: expert code reviewer role, `gh pr list` / `gh pr view` / `gh pr diff` steps, review dimensions covering correctness, conventions, performance, test coverage, security.)

---

## [review.ts]
### /ultrareview command description (user-facing)
**File:** `src/commands/review.ts:48-53`
```ts
const ultrareview: Command = {
  type: 'local-jsx',
  name: 'ultrareview',
  description: `~10-20 min. Finds and verifies bugs in your branch. Runs in Claude Code on the web. See ${CCR_TERMS_URL}`,
  // ...
}
```
**Status: NOT IN RUST** -- Reason: The /ultrareview in Rust (`crates/claude-core/src/commands/builtin.rs:1438`) is a local deep-review command. The remote ultrareview (CCR cloud execution) description with `CCR_TERMS_URL` is not ported because the remote/cloud execution infrastructure does not exist in the Rust port.

---

## [review/reviewRemote.ts]
### Ultrareview launch success message (injected into conversation for model)
**File:** `src/commands/review/reviewRemote.ts:310-315`
```ts
return [
  {
    type: 'text',
    text: `Ultrareview launched for ${target} (~10-20 min, runs in the cloud). Track: ${sessionUrl}${resolvedBillingNote} Findings arrive via task-notification. Briefly acknowledge the launch to the user without repeating the target or URL -- both are already visible in the tool output above.`,
  },
]
```
**Status: NOT IN RUST** -- Reason: Remote ultrareview (CCR cloud execution) is not implemented in the Rust port. The entire reviewRemote.ts infrastructure (cloud session launch, billing, task-notification) does not exist.

---

## [security-review.ts]
### /security-review command prompt - Full security review with methodology
**File:** `src/commands/security-review.ts:6-196`

This is a very large prompt (~5000 words). It includes:
- Role: "You are a senior security engineer conducting a focused security review of the changes on this branch."
- Git context injection via shell commands (`!\`git status\``, `!\`git diff origin/HEAD...\``, etc.)
- Security categories to examine (Input Validation, Auth, Crypto, Injection, Data Exposure)
- Analysis methodology in 3 phases (Repository Context Research, Comparative Analysis, Vulnerability Assessment)
- Required output format with severity/confidence scoring
- Extensive false positive filtering rules (17 hard exclusions, 12 precedents, 4 signal quality criteria)
- Multi-agent execution: "Begin your analysis now. Do this in 3 steps: 1. Use a sub-task to identify vulnerabilities. 2. Then for each vulnerability identified by the above sub-task, create a new sub-task to filter out false-positives. Launch these sub-tasks as parallel sub-tasks. 3. Filter out any vulnerabilities where the sub-task reported a confidence less than 8."

Full source is at `src/commands/security-review.ts:6-196` (the `SECURITY_REVIEW_MARKDOWN` constant).
**Status: ADDED to Rust** -- `crates/claude-core/src/commands/builtin.rs:2064` (SecurityReviewHandler updated with multi-agent execution hint from TS: "Begin your analysis now. Do this in 3 steps..." with sub-task parallelization for vulnerability identification and false-positive filtering. Core categories, severity guidelines, and false-positive exclusions were already present. Note: the TS version is ~5000 words with 17 hard exclusions and 12 precedents; the Rust version is a condensed but comprehensive version covering the essential security review methodology.)

---

## [init.ts]
### /init command prompt - OLD_INIT_PROMPT (legacy CLAUDE.md initialization)
**File:** `src/commands/init.ts:6-26`
```ts
const OLD_INIT_PROMPT = `Please analyze this codebase and create a CLAUDE.md file, which will be given to future instances of Claude Code to operate in this repository.

What to add:
1. Commands that will be commonly used, such as how to build, lint, and run tests. Include the necessary commands to develop in this codebase, such as how to run a single test.
2. High-level code architecture and structure so that future instances can be productive more quickly. Focus on the "big picture" architecture that requires reading multiple files to understand.

Usage notes:
- If there's already a CLAUDE.md, suggest improvements to it.
- When you make the initial CLAUDE.md, do not repeat yourself and do not include obvious instructions like "Provide helpful error messages to users", "Write unit tests for all new utilities", "Never include sensitive information (API keys, tokens) in code or commits".
- Avoid listing every component or file structure that can be easily discovered.
- Don't include generic development practices.
- If there are Cursor rules (in .cursor/rules/ or .cursorrules) or Copilot rules (in .github/copilot-instructions.md), make sure to include the important parts.
- If there is a README.md, make sure to include the important parts.
- Do not make up information such as "Common Development Tasks", "Tips for Development", "Support and Documentation" unless this is expressly included in other files that you read.
- Be sure to prefix the file with the following text:

\`\`\`
# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.
\`\`\``
```
**Status: ADDED to Rust** -- `crates/claude-core/src/commands/builtin.rs:864` (INIT_PROMPT const added matching the TS OLD_INIT_PROMPT. InitHandler converted from Action to Prompt command type so it returns the prompt to the model for codebase analysis. Includes all guidance: build/lint/test commands, architecture, usage notes about .cursorrules, README.md, avoiding generic practices, and the CLAUDE.md prefix template.)

---

## [init.ts]
### /init command prompt - NEW_INIT_PROMPT (multi-phase CLAUDE.md + skills + hooks initialization)
**File:** `src/commands/init.ts:28-224`

This is a very large prompt (~6000+ words) with 8 phases:

- **Phase 1**: Ask what to set up (Project CLAUDE.md / Personal CLAUDE.local.md / Both; Skills + hooks / Skills only / Hooks only / Neither)
- **Phase 2**: Explore the codebase (launch subagent to survey manifest files, README, CI config, existing CLAUDE.md, etc.)
- **Phase 3**: Fill in the gaps (AskUserQuestion for codebase practices, personal preferences, synthesize proposals)
- **Phase 4**: Write CLAUDE.md (minimal, pass the test: "Would removing this cause Claude to make mistakes?")
- **Phase 5**: Write CLAUDE.local.md (personal preferences, gitignored)
- **Phase 6**: Suggest and create skills (on-demand capabilities at `.claude/skills/<skill-name>/SKILL.md`)
- **Phase 7**: Suggest additional optimizations (GitHub CLI, linting, hooks via the `update-config` skill)
- **Phase 8**: Summary and next steps (recap + to-do list of further optimizations)

Full source is at `src/commands/init.ts:28-224` (the `NEW_INIT_PROMPT` constant).
**Status: NOT IN RUST** -- Reason: The NEW_INIT_PROMPT is a ~6000+ word multi-phase wizard (8 phases: setup choice, codebase exploration via subagent, gap-filling via AskUserQuestion, CLAUDE.md writing, CLAUDE.local.md, skills creation, optimization suggestions, summary). This requires infrastructure not yet in the Rust port: subagent spawning from commands, AskUserQuestion tool integration from commands, skills creation workflow, and the `update-config` skill. The Rust port uses the simpler OLD_INIT_PROMPT instead.

---

## [init-verifiers.ts]
### /init-verifiers command prompt - Verifier skill creation wizard
**File:** `src/commands/init-verifiers.ts:15-256`

This is a large prompt (~3000+ words) with 5 phases:

- **Phase 1**: Auto-Detection (scan project for application types, frameworks, dev servers)
- **Phase 2**: Verification Tool Setup (Playwright, Chrome DevTools MCP, CLI/Tmux, API/HTTP)
- **Phase 3**: Interactive Q&A (verifier name, project-specific questions, authentication/login)
- **Phase 4**: Generate Verifier Skill (template with Project Context, Setup Instructions, Auth, Reporting, Cleanup, Self-Update sections)
- **Phase 5**: Confirm Creation

Includes a skill template structure:
```markdown
# <Verifier Title>

You are a verification executor. You receive a verification plan and execute it EXACTLY as written.

## Project Context
<Project-specific details from detection>

## Setup Instructions
<How to start any required services>

## Reporting
Report PASS or FAIL for each step using the format specified in the verification plan.

## Cleanup
After verification:
1. Stop any dev servers started
2. Close any browser sessions
3. Report final summary

## Self-Update
If verification fails because this skill's instructions are outdated [...]
use AskUserQuestion to confirm and then Edit this SKILL.md with a minimal targeted fix.
```

Full source is at `src/commands/init-verifiers.ts:15-256`.
**Status: NOT IN RUST** -- Reason: The /init-verifiers command is not implemented in the Rust port. It requires a multi-phase wizard with auto-detection of application types/frameworks, verification tool setup (Playwright, Chrome DevTools MCP, CLI/Tmux), interactive Q&A, and skill template generation. This infrastructure does not exist in Rust.

---

## [rename/generateSessionName.ts]
### Session name generation prompt (sent to Haiku)
**File:** `src/commands/rename/generateSessionName.ts:20-23`
```ts
const result = await queryHaiku({
  systemPrompt: asSystemPrompt([
    'Generate a short kebab-case name (2-4 words) that captures the main topic of this conversation. Use lowercase words separated by hyphens. Examples: "fix-login-bug", "add-auth-feature", "refactor-api-client", "debug-test-failures". Return JSON with a "name" field.',
  ]),
  userPrompt: conversationText,
  outputFormat: {
    type: 'json_schema',
    schema: {
      type: 'object',
      properties: {
        name: { type: 'string' },
      },
      required: ['name'],
      additionalProperties: false,
    },
  },
  // ...
})
```
**Status: FOUND in Rust (prompt-only)** -- `crates/claude-core/src/command_prompt_snippets.rs::SESSION_NAME_GENERATION_SYSTEM_PROMPT`. Haiku query runtime still requires the secondary-model query API — the prompt text ships in the binary.

---

## [insights.ts]
### SUMMARIZE_CHUNK_PROMPT - Transcript chunk summarization
**File:** `src/commands/insights.ts:870-878`
```ts
const SUMMARIZE_CHUNK_PROMPT = `Summarize this portion of a Claude Code session transcript. Focus on:
1. What the user asked for
2. What Claude did (tools used, files modified)
3. Any friction or issues
4. The outcome

Keep it concise - 3-5 sentences. Preserve specific details like file names, error messages, and user feedback.

TRANSCRIPT CHUNK:
`
```
**Status: FOUND in Rust (prompt-only)** -- `crates/claude-core/src/insights_prompts.rs::SUMMARIZE_CHUNK_PROMPT`. The multi-step pipeline infrastructure (chunking + sequential model queries) is still deferred; the text ships for when it lands.

---

## [insights.ts]
### FACET_EXTRACTION_PROMPT - Session analysis and facet extraction
**File:** `src/commands/insights.ts:430-456`
```ts
const FACET_EXTRACTION_PROMPT = `Analyze this Claude Code session and extract structured facets.

CRITICAL GUIDELINES:

1. **goal_categories**: Count ONLY what the USER explicitly asked for.
   - DO NOT count Claude's autonomous codebase exploration
   - DO NOT count work Claude decided to do on its own
   - ONLY count when user says "can you...", "please...", "I need...", "let's..."

2. **user_satisfaction_counts**: Base ONLY on explicit user signals.
   - "Yay!", "great!", "perfect!" -> happy
   - "thanks", "looks good", "that works" -> satisfied
   - "ok, now let's..." (continuing without complaint) -> likely_satisfied
   - "that's not right", "try again" -> dissatisfied
   - "this is broken", "I give up" -> frustrated

3. **friction_counts**: Be specific about what went wrong.
   - misunderstood_request: Claude interpreted incorrectly
   - wrong_approach: Right goal, wrong solution method
   - buggy_code: Code didn't work correctly
   - user_rejected_action: User said no/stop to a tool call
   - excessive_changes: Over-engineered or changed too much

4. If very short or just warmup, use warmup_minimal for goal_category

SESSION:
`
```
**Status: FOUND in Rust (prompt-only)** -- `crates/claude-core/src/insights_prompts.rs::FACET_EXTRACTION_PROMPT`.

---

## [insights.ts]
### Facet extraction JSON response format prompt
**File:** `src/commands/insights.ts:1010-1024`
```ts
const jsonPrompt = `${FACET_EXTRACTION_PROMPT}${transcript}

RESPOND WITH ONLY A VALID JSON OBJECT matching this schema:
{
  "underlying_goal": "What the user fundamentally wanted to achieve",
  "goal_categories": {"category_name": count, ...},
  "outcome": "fully_achieved|mostly_achieved|partially_achieved|not_achieved|unclear_from_transcript",
  "user_satisfaction_counts": {"level": count, ...},
  "claude_helpfulness": "unhelpful|slightly_helpful|moderately_helpful|very_helpful|essential",
  "session_type": "single_task|multi_task|iterative_refinement|exploration|quick_question",
  "friction_counts": {"friction_type": count, ...},
  "friction_detail": "One sentence describing friction or empty",
  "primary_success": "none|fast_accurate_search|correct_code_edits|good_explanations|proactive_help|multi_file_changes|good_debugging",
  "brief_summary": "One sentence: what user wanted and whether they got it"
}`
```
**Status: FOUND in Rust (prompt-only)** -- `crates/claude-core/src/insights_prompts.rs::FACET_EXTRACTION_JSON_SUFFIX` + `facet_extraction_json_prompt()`.

---

## [insights.ts]
### INSIGHT_SECTIONS - Project areas analysis prompt
**File:** `src/commands/insights.ts:1337-1348`
```ts
{
  name: 'project_areas',
  prompt: `Analyze this Claude Code usage data and identify project areas.

RESPOND WITH ONLY A VALID JSON OBJECT:
{
  "areas": [
    {"name": "Area name", "session_count": N, "description": "2-3 sentences about what was worked on and how Claude Code was used."}
  ]
}

Include 4-5 areas. Skip internal CC operations.`,
  maxTokens: 8192,
},
```
**Status: FOUND in Rust (prompt-only)** -- `crates/claude-core/src/insights_prompts.rs::INSIGHT_SECTION_PROJECT_AREAS`. Per-section model-query runtime still deferred.

---

## [insights.ts]
### INSIGHT_SECTIONS - Interaction style analysis prompt
**File:** `src/commands/insights.ts:1351-1360`
```ts
{
  name: 'interaction_style',
  prompt: `Analyze this Claude Code usage data and describe the user's interaction style.

RESPOND WITH ONLY A VALID JSON OBJECT:
{
  "narrative": "2-3 paragraphs analyzing HOW the user interacts with Claude Code. Use second person 'you'. Describe patterns: iterate quickly vs detailed upfront specs? Interrupt often or let Claude run? Include specific examples. Use **bold** for key insights.",
  "key_pattern": "One sentence summary of most distinctive interaction style"
}`,
  maxTokens: 8192,
},
```
**Status: FOUND in Rust (prompt-only)** -- `crates/claude-core/src/insights_prompts.rs::INSIGHT_SECTION_INTERACTION_STYLE`.

---

## [insights.ts]
### INSIGHT_SECTIONS - What works well prompt
**File:** `src/commands/insights.ts:1362-1375`
```ts
{
  name: 'what_works',
  prompt: `Analyze this Claude Code usage data and identify what's working well for this user. Use second person ("you").

RESPOND WITH ONLY A VALID JSON OBJECT:
{
  "intro": "1 sentence of context",
  "impressive_workflows": [
    {"title": "Short title (3-6 words)", "description": "2-3 sentences describing the impressive workflow or approach. Use 'you' not 'the user'."}
  ]
}

Include 3 impressive workflows.`,
  maxTokens: 8192,
},
```
**Status: FOUND in Rust (prompt-only)** -- `crates/claude-core/src/insights_prompts.rs::INSIGHT_SECTION_WHAT_WORKS`.

---

## [insights.ts]
### INSIGHT_SECTIONS - Friction analysis prompt
**File:** `src/commands/insights.ts:1377-1390`
```ts
{
  name: 'friction_analysis',
  prompt: `Analyze this Claude Code usage data and identify friction points for this user. Use second person ("you").

RESPOND WITH ONLY A VALID JSON OBJECT:
{
  "intro": "1 sentence summarizing friction patterns",
  "categories": [
    {"category": "Concrete category name", "description": "1-2 sentences explaining this category and what could be done differently. Use 'you' not 'the user'.", "examples": ["Specific example with consequence", "Another example"]}
  ]
}

Include 3 friction categories with 2 examples each.`,
  maxTokens: 8192,
},
```
**Status: FOUND in Rust (prompt-only)** -- `crates/claude-core/src/insights_prompts.rs::INSIGHT_SECTION_FRICTION_ANALYSIS`.

---

## [insights.ts]
### INSIGHT_SECTIONS - Suggestions prompt (with CC features reference)
**File:** `src/commands/insights.ts:1392-1433`
```ts
{
  name: 'suggestions',
  prompt: `Analyze this Claude Code usage data and suggest improvements.

## CC FEATURES REFERENCE (pick from these for features_to_try):
1. **MCP Servers**: Connect Claude to external tools, databases, and APIs via Model Context Protocol.
   - How to use: Run \`claude mcp add <server-name> -- <command>\`
   - Good for: database queries, Slack integration, GitHub issue lookup, connecting to internal APIs

2. **Custom Skills**: Reusable prompts you define as markdown files that run with a single /command.
   - How to use: Create \`.claude/skills/commit/SKILL.md\` with instructions. Then type \`/commit\` to run it.
   - Good for: repetitive workflows - /commit, /review, /test, /deploy, /pr, or complex multi-step workflows

3. **Hooks**: Shell commands that auto-run at specific lifecycle events.
   - How to use: Add to \`.claude/settings.json\` under "hooks" key.
   - Good for: auto-formatting code, running type checks, enforcing conventions

4. **Headless Mode**: Run Claude non-interactively from scripts and CI/CD.
   - How to use: \`claude -p "fix lint errors" --allowedTools "Edit,Read,Bash"\`
   - Good for: CI/CD integration, batch code fixes, automated reviews

5. **Task Agents**: Claude spawns focused sub-agents for complex exploration or parallel work.
   - How to use: Claude auto-invokes when helpful, or ask "use an agent to explore X"
   - Good for: codebase exploration, understanding complex systems

RESPOND WITH ONLY A VALID JSON OBJECT:
{
  "claude_md_additions": [
    {"addition": "A specific line or block to add to CLAUDE.md based on workflow patterns...", "why": "1 sentence...", "prompt_scaffold": "Instructions for where to add this in CLAUDE.md..."}
  ],
  "features_to_try": [
    {"feature": "Feature name from CC FEATURES REFERENCE above", "one_liner": "What it does", "why_for_you": "Why this would help YOU based on your sessions", "example_code": "Actual command or config to copy"}
  ],
  "usage_patterns": [
    {"title": "Short title", "suggestion": "1-2 sentence summary", "detail": "3-4 sentences explaining how this applies to YOUR work", "copyable_prompt": "A specific prompt to copy and try"}
  ]
}

IMPORTANT for claude_md_additions: PRIORITIZE instructions that appear MULTIPLE TIMES in the user data. If user told Claude the same thing in 2+ sessions (e.g., 'always run tests', 'use TypeScript'), that's a PRIME candidate - they shouldn't have to repeat themselves.

IMPORTANT for features_to_try: Pick 2-3 from the CC FEATURES REFERENCE above. Include 2-3 items for each category.`,
  maxTokens: 8192,
},
```
**Status: FOUND in Rust (prompt-only)** -- `crates/claude-core/src/insights_prompts.rs::INSIGHT_SECTION_SUGGESTIONS`.

---

## [insights.ts]
### INSIGHT_SECTIONS - On the horizon prompt
**File:** `src/commands/insights.ts:1435-1448`
```ts
{
  name: 'on_the_horizon',
  prompt: `Analyze this Claude Code usage data and identify future opportunities.

RESPOND WITH ONLY A VALID JSON OBJECT:
{
  "intro": "1 sentence about evolving AI-assisted development",
  "opportunities": [
    {"title": "Short title (4-8 words)", "whats_possible": "2-3 ambitious sentences about autonomous workflows", "how_to_try": "1-2 sentences mentioning relevant tooling", "copyable_prompt": "Detailed prompt to try"}
  ]
}

Include 3 opportunities. Think BIG - autonomous workflows, parallel agents, iterating against tests.`,
  maxTokens: 8192,
},
```
**Status: FOUND in Rust (prompt-only)** -- `crates/claude-core/src/insights_prompts.rs::INSIGHT_SECTION_ON_THE_HORIZON`.

---

## [insights.ts]
### INSIGHT_SECTIONS - CC team improvements prompt (ant-only)
**File:** `src/commands/insights.ts:1452-1465`
```ts
{
  name: 'cc_team_improvements',
  prompt: `Analyze this Claude Code usage data and suggest product improvements for the CC team.

RESPOND WITH ONLY A VALID JSON OBJECT:
{
  "improvements": [
    {"title": "Product/tooling improvement", "detail": "3-4 sentences describing the improvement", "evidence": "3-4 sentences with specific session examples"}
  ]
}

Include 2-3 improvements based on friction patterns observed.`,
  maxTokens: 8192,
},
```
**Status: FOUND in Rust (prompt-only)** -- `crates/claude-core/src/insights_prompts.rs::INSIGHT_SECTION_CC_TEAM_IMPROVEMENTS` (ant-only caller-gated).

---

## [insights.ts]
### INSIGHT_SECTIONS - Model behavior improvements prompt (ant-only)
**File:** `src/commands/insights.ts:1467-1479`
```ts
{
  name: 'model_behavior_improvements',
  prompt: `Analyze this Claude Code usage data and suggest model behavior improvements.

RESPOND WITH ONLY A VALID JSON OBJECT:
{
  "improvements": [
    {"title": "Model behavior change", "detail": "3-4 sentences describing what the model should do differently", "evidence": "3-4 sentences with specific examples"}
  ]
}

Include 2-3 improvements based on friction patterns observed.`,
  maxTokens: 8192,
},
```
**Status: FOUND in Rust (prompt-only)** -- `crates/claude-core/src/insights_prompts.rs::INSIGHT_SECTION_MODEL_BEHAVIOR_IMPROVEMENTS` (ant-only caller-gated).

---

## [insights.ts]
### INSIGHT_SECTIONS - Fun ending prompt
**File:** `src/commands/insights.ts:1482-1494`
```ts
{
  name: 'fun_ending',
  prompt: `Analyze this Claude Code usage data and find a memorable moment.

RESPOND WITH ONLY A VALID JSON OBJECT:
{
  "headline": "A memorable QUALITATIVE moment from the transcripts - not a statistic. Something human, funny, or surprising.",
  "detail": "Brief context about when/where this happened"
}

Find something genuinely interesting or amusing from the session summaries.`,
  maxTokens: 8192,
},
```
**Status: FOUND in Rust (prompt-only)** -- `crates/claude-core/src/insights_prompts.rs::INSIGHT_SECTION_FUN_ENDING`.

---

## [insights.ts]
### At a Glance summary prompt (synthesizes other sections)
**File:** `src/commands/insights.ts:1738-1779`
```ts
const atAGlancePrompt = `You're writing an "At a Glance" summary for a Claude Code usage insights report for Claude Code users. The goal is to help them understand their usage and improve how they can use Claude better, especially as models improve.

Use this 4-part structure:

1. **What's working** - What is the user's unique style of interacting with Claude and what are some impactful things they've done? You can include one or two details, but keep it high level since things might not be fresh in the user's memory. Don't be fluffy or overly complimentary. Also, don't focus on the tool calls they use.

2. **What's hindering you** - Split into (a) Claude's fault (misunderstandings, wrong approaches, bugs) and (b) user-side friction (not providing enough context, environment issues -- ideally more general than just one project). Be honest but constructive.

3. **Quick wins to try** - Specific Claude Code features they could try from the examples below, or a workflow technique if you think it's really compelling. (Avoid stuff like "Ask Claude to confirm before taking actions" or "Type out more context up front" which are less compelling.)

4. **Ambitious workflows for better models** - As we move to much more capable models over the next 3-6 months, what should they prepare for? What workflows that seem impossible now will become possible? Draw from the appropriate section below.

Keep each section to 2-3 not-too-long sentences. Don't overwhelm the user. Don't mention specific numerical stats or underlined_categories from the session data below. Use a coaching tone.

RESPOND WITH ONLY A VALID JSON OBJECT:
{
  "whats_working": "(refer to instructions above)",
  "whats_hindering": "(refer to instructions above)",
  "quick_wins": "(refer to instructions above)",
  "ambitious_workflows": "(refer to instructions above)"
}

SESSION DATA:
${fullContext}

## Project Areas (what user works on)
${projectAreasText}

## Big Wins (impressive accomplishments)
${bigWinsText}

## Friction Categories (where things go wrong)
${frictionText}

## Features to Try
${featuresText}

## Usage Patterns to Adopt
${patternsText}

## On the Horizon (ambitious workflows for better models)
${horizonText}`
```
**Status: FOUND in Rust (prompt-only)** -- `crates/claude-core/src/insights_prompts.rs::at_a_glance_prompt()` with `AtAGlanceInputs` filling the seven dynamic slots.

---

## [insights.ts]
### /insights final prompt returned to model
**File:** `src/commands/insights.ts:3156-3180`
```ts
return [
  {
    type: 'text',
    text: `The user just ran /insights to generate a usage report analyzing their Claude Code sessions.

Here is the full insights data:
${jsonStringify(insights, null, 2)}

Report URL: ${reportUrl}
HTML file: ${htmlPath}
Facets directory: ${getFacetsDir()}

Here is what the user sees:
${userSummary}

Now output the following message exactly:

<message>
Your shareable insights report is ready:
${reportUrl}${uploadHint}

Want to dig into any section or try one of the suggestions?
</message>`,
  },
]
```
**Status: FOUND in Rust (prompt-only)** -- `crates/claude-core/src/insights_prompts.rs::insights_final_report_message()` with URL / HTML path / facets dir / user summary / upload hint arguments.

---

## [statusline.tsx]
### /statusline command prompt
**File:** `src/commands/statusline.tsx:14-19`
```ts
async getPromptForCommand(args): Promise<ContentBlockParam[]> {
  const prompt = args.trim() || 'Configure my statusLine from my shell PS1 configuration'
  return [{
    type: 'text',
    text: `Create an ${AGENT_TOOL_NAME} with subagent_type "statusline-setup" and the prompt "${prompt}"`
  }];
}
```
**Status: FOUND in Rust (prompt-only)** -- `crates/claude-core/src/command_prompt_snippets.rs::statusline_command_prompt` + `STATUSLINE_DEFAULT_PROMPT`. Subagent dispatch (spawning `statusline-setup` agent) still deferred — the `statusline-setup` built-in agent itself is already registered in `claude-tools/src/agents/`.

---

## [pr_comments/index.ts]
### /pr-comments command prompt
**File:** `src/commands/pr_comments/index.ts:11-47`
```ts
return [
  {
    type: 'text',
    text: `You are an AI assistant integrated into a git-based version control system. Your task is to fetch and display comments from a GitHub pull request.

Follow these steps:

1. Use \`gh pr view --json number,headRepository\` to get the PR number and repository info
2. Use \`gh api /repos/{owner}/{repo}/issues/{number}/comments\` to get PR-level comments
3. Use \`gh api /repos/{owner}/{repo}/pulls/{number}/comments\` to get review comments. Pay particular attention to the following fields: \`body\`, \`diff_hunk\`, \`path\`, \`line\`, etc. If the comment references some code, consider fetching it using eg \`gh api /repos/{owner}/{repo}/contents/{path}?ref={branch} | jq .content -r | base64 -d\`
4. Parse and format all comments in a readable way
5. Return ONLY the formatted comments, with no additional text

Format the comments as:

## Comments

[For each comment thread:]
- @author file.ts#line:
  \`\`\`diff
  [diff_hunk from the API response]
  \`\`\`
  > quoted comment text

  [any replies indented]

If there are no comments, return "No comments found."

Remember:
1. Only show the actual comments, no explanatory text
2. Include both PR-level and code review comments
3. Preserve the threading/nesting of comment replies
4. Show the file and line number context for code review comments
5. Use jq to parse the JSON responses from the GitHub API

${args ? 'Additional user input: ' + args : ''}
`,
  },
]
```
**Status: ADDED to Rust** -- `crates/claude-core/src/commands/builtin.rs:1381` (PrCommentsHandler updated with full TS prompt including all 5 steps: gh pr view, gh api for issue comments, gh api for review comments with diff_hunk/path/line, parse/format, and the detailed output format template with @author, diff hunks, quoted comments, threading. Also includes the 5 "Remember" items and jq instructions.)

---

## [createMovedToPluginCommand.ts]
### "Moved to plugin" redirect prompt (ant-only)
**File:** `src/commands/createMovedToPluginCommand.ts:44-57`
```ts
if (process.env.USER_TYPE === 'ant') {
  return [
    {
      type: 'text',
      text: `This command has been moved to a plugin. Tell the user:

1. To install the plugin, run:
   claude plugin install ${pluginName}@claude-code-marketplace

2. After installation, use /${pluginName}:${pluginCommand} to run this command

3. For more information, see: https://github.com/anthropics/claude-code-marketplace/blob/main/${pluginName}/README.md

Do not attempt to run the command. Simply inform the user about the plugin installation.`,
    },
  ]
}
```
**Status: FOUND in Rust (prompt-only)** -- `crates/claude-core/src/command_prompt_snippets.rs::moved_to_plugin_redirect(plugin_name, plugin_command)`. Caller still owns the `USER_TYPE === 'ant'` gate.

---

## [brief.ts]
### Brief mode toggle system reminder (injected as metaMessage)
**File:** `src/commands/brief.ts:111-118`
```ts
const metaMessages = getKairosActive()
  ? undefined
  : [
      `<system-reminder>\n${
        newState
          ? `Brief mode is now enabled. Use the ${BRIEF_TOOL_NAME} tool for all user-facing output -- plain text outside it is hidden from the user's view.`
          : `Brief mode is now disabled. The ${BRIEF_TOOL_NAME} tool is no longer available -- reply with plain text.`
      }\n</system-reminder>`,
    ]
```
**Status: FOUND in Rust** -- `crates/claude-tools/src/brief_tool.rs:44-73` (The brief mode toggle prompts exist as BRIEF_SYSTEM_PROMPT_SECTION, BRIEF_ENABLED_INSTRUCTIONS, and BRIEF_DISABLED_INSTRUCTIONS. The TS version injects a `<system-reminder>` metaMessage while the Rust version uses tool result data and system prompt section injection. The core content matches: "Brief mode is now ON/OFF" with the same brevity rules. Note: The TS wraps in `<system-reminder>` tags and references BRIEF_TOOL_NAME; the Rust version achieves the same effect via the system prompt builder calling `get_brief_system_prompt_section()`.)

---

## [ultraplan.tsx]
### /ultraplan prompt assembly (prompt.txt loaded at runtime, not in leaked source)
**File:** `src/commands/ultraplan.tsx:45-71`
```ts
// prompt.txt is wrapped in <system-reminder> so the CCR browser hides
// scaffolding (CLI_BLOCK_TAGS dropped by stripSystemNotifications)
// while the model still sees full text.
const _rawPrompt = require('../utils/ultraplan/prompt.txt');
const DEFAULT_INSTRUCTIONS: string = (typeof _rawPrompt === 'string' ? _rawPrompt : _rawPrompt.default).trimEnd();

// Dev-only prompt override resolved eagerly at module load.
const ULTRAPLAN_INSTRUCTIONS: string = "external" === 'ant' && process.env.ULTRAPLAN_PROMPT_FILE
  ? readFileSync(process.env.ULTRAPLAN_PROMPT_FILE, 'utf8').trimEnd()
  : DEFAULT_INSTRUCTIONS;

/**
 * Assemble the initial CCR user message. seedPlan and blurb stay outside the
 * system-reminder so the browser renders them; scaffolding is hidden.
 */
export function buildUltraplanPrompt(blurb: string, seedPlan?: string): string {
  const parts: string[] = [];
  if (seedPlan) {
    parts.push('Here is a draft plan to refine:', '', seedPlan, '');
  }
  parts.push(ULTRAPLAN_INSTRUCTIONS);
  if (blurb) {
    parts.push('', blurb);
  }
  return parts.join('\n');
}
```
(Note: The actual `prompt.txt` content is not present in the leaked source -- it was likely bundled at build time and stripped.)
**Status: FOUND in Rust** -- `crates/claude-core/src/commands/builtin.rs:2109` (UltraplanHandler exists with a comprehensive planning prompt. Since the TS prompt.txt content was not available in the source, the Rust version provides its own ultra-detailed planning prompt covering objective, sub-tasks, dependencies, risks, complexity estimation, and acceptance criteria. The buildUltraplanPrompt assembly function concept is not needed since Rust handles it directly.)

---

# Hooks Directory

The `/src/hooks/` directory contains React hooks for UI state management, input handling, notifications, and permission flows. After thorough examination of all 90+ files, **no LLM prompts, system messages, or prompt templates were found** in this directory. The hooks directory is purely UI/state logic:

- `usePromptSuggestion.ts` - Manages prompt suggestion display state (Tab to accept), no LLM prompt content
- `usePromptsFromClaudeInChrome.tsx` - Relays prompts from Chrome extension to the message queue, no prompt construction
- `useAwaySummary.ts` - Triggers away-summary generation but delegates to `services/awaySummary.ts` (outside this directory)
- `renderPlaceholder.ts` - Renders input placeholder text styling, no LLM content
- `fileSuggestions.ts` / `unifiedSuggestions.ts` - File path typeahead using Rust/nucleo indexing, no LLM content
- `toolPermission/` handlers - Permission UI flow logic, no prompts
- All `notifs/` hooks - UI notification display logic, no LLM content

**Status: N/A** -- The TS `/src/hooks/` directory contains React UI hooks, not LLM prompts. The Rust equivalent is `crates/claude-core/src/hooks/` which implements the hook execution engine (matching, running, aggregation) -- this is the settings.json hooks system, not UI hooks. The Rust hooks implementation (`crates/claude-core/src/hooks/`) is complete with types.rs (27 hook events, 4 command types), runner.rs (execution engine), matching.rs (pattern matching), tool_hooks.rs (PreToolUse/PostToolUse integration), and aggregation.rs. No additional prompts needed from this section.
