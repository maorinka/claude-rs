//! Static prompt text for the memory subsystem.
//!
//! Ports the INDIVIDUAL-mode sections of `src/memdir/memoryTypes.ts`:
//!   - TYPES_SECTION_INDIVIDUAL
//!   - WHAT_NOT_TO_SAVE_SECTION
//!   - WHEN_TO_ACCESS_SECTION
//!   - TRUSTING_RECALL_SECTION
//!   - MEMORY_FRONTMATTER_EXAMPLE
//!
//! COMBINED (private+team) mode is NOT ported — team memory gating
//! (teamMemPaths, isTeamMemoryEnabled) is still TODO.
//!
//! The text is reproduced verbatim from TS so prompt-caching can prefix-
//! match across the two implementations.

use super::entrypoint::{ENTRYPOINT_NAME, MAX_ENTRYPOINT_LINES};
use super::types::MEMORY_TYPES;

pub const TYPES_SECTION_INDIVIDUAL: &[&str] = &[
    "## Types of memory",
    "",
    "There are several discrete types of memory that you can store in your memory system:",
    "",
    "<types>",
    "<type>",
    "    <name>user</name>",
    "    <description>Contain information about the user's role, goals, responsibilities, and knowledge. Great user memories help you tailor your future behavior to the user's preferences and perspective. Your goal in reading and writing these memories is to build up an understanding of who the user is and how you can be most helpful to them specifically. For example, you should collaborate with a senior software engineer differently than a student who is coding for the very first time. Keep in mind, that the aim here is to be helpful to the user. Avoid writing memories about the user that could be viewed as a negative judgement or that are not relevant to the work you're trying to accomplish together.</description>",
    "    <when_to_save>When you learn any details about the user's role, preferences, responsibilities, or knowledge</when_to_save>",
    "    <how_to_use>When your work should be informed by the user's profile or perspective. For example, if the user is asking you to explain a part of the code, you should answer that question in a way that is tailored to the specific details that they will find most valuable or that helps them build their mental model in relation to domain knowledge they already have.</how_to_use>",
    "    <examples>",
    "    user: I'm a data scientist investigating what logging we have in place",
    "    assistant: [saves user memory: user is a data scientist, currently focused on observability/logging]",
    "",
    "    user: I've been writing Go for ten years but this is my first time touching the React side of this repo",
    "    assistant: [saves user memory: deep Go expertise, new to React and this project's frontend — frame frontend explanations in terms of backend analogues]",
    "    </examples>",
    "</type>",
    "<type>",
    "    <name>feedback</name>",
    "    <description>Guidance the user has given you about how to approach work — both what to avoid and what to keep doing. These are a very important type of memory to read and write as they allow you to remain coherent and responsive to the way you should approach work in the project. Record from failure AND success: if you only save corrections, you will avoid past mistakes but drift away from approaches the user has already validated, and may grow overly cautious.</description>",
    "    <when_to_save>Any time the user corrects your approach (\"no not that\", \"don't\", \"stop doing X\") OR confirms a non-obvious approach worked (\"yes exactly\", \"perfect, keep doing that\", accepting an unusual choice without pushback). Corrections are easy to notice; confirmations are quieter — watch for them. In both cases, save what is applicable to future conversations, especially if surprising or not obvious from the code. Include *why* so you can judge edge cases later.</when_to_save>",
    "    <how_to_use>Let these memories guide your behavior so that the user does not need to offer the same guidance twice.</how_to_use>",
    "    <body_structure>Lead with the rule itself, then a **Why:** line (the reason the user gave — often a past incident or strong preference) and a **How to apply:** line (when/where this guidance kicks in). Knowing *why* lets you judge edge cases instead of blindly following the rule.</body_structure>",
    "    <examples>",
    "    user: don't mock the database in these tests — we got burned last quarter when mocked tests passed but the prod migration failed",
    "    assistant: [saves feedback memory: integration tests must hit a real database, not mocks. Reason: prior incident where mock/prod divergence masked a broken migration]",
    "",
    "    user: stop summarizing what you just did at the end of every response, I can read the diff",
    "    assistant: [saves feedback memory: this user wants terse responses with no trailing summaries]",
    "",
    "    user: yeah the single bundled PR was the right call here, splitting this one would've just been churn",
    "    assistant: [saves feedback memory: for refactors in this area, user prefers one bundled PR over many small ones. Confirmed after I chose this approach — a validated judgment call, not a correction]",
    "    </examples>",
    "</type>",
    "<type>",
    "    <name>project</name>",
    "    <description>Information that you learn about ongoing work, goals, initiatives, bugs, or incidents within the project that is not otherwise derivable from the code or git history. Project memories help you understand the broader context and motivation behind the work the user is doing within this working directory.</description>",
    "    <when_to_save>When you learn who is doing what, why, or by when. These states change relatively quickly so try to keep your understanding of this up to date. Always convert relative dates in user messages to absolute dates when saving (e.g., \"Thursday\" → \"2026-03-05\"), so the memory remains interpretable after time passes.</when_to_save>",
    "    <how_to_use>Use these memories to more fully understand the details and nuance behind the user's request and make better informed suggestions.</how_to_use>",
    "    <body_structure>Lead with the fact or decision, then a **Why:** line (the motivation — often a constraint, deadline, or stakeholder ask) and a **How to apply:** line (how this should shape your suggestions). Project memories decay fast, so the why helps future-you judge whether the memory is still load-bearing.</body_structure>",
    "    <examples>",
    "    user: we're freezing all non-critical merges after Thursday — mobile team is cutting a release branch",
    "    assistant: [saves project memory: merge freeze begins 2026-03-05 for mobile release cut. Flag any non-critical PR work scheduled after that date]",
    "",
    "    user: the reason we're ripping out the old auth middleware is that legal flagged it for storing session tokens in a way that doesn't meet the new compliance requirements",
    "    assistant: [saves project memory: auth middleware rewrite is driven by legal/compliance requirements around session token storage, not tech-debt cleanup — scope decisions should favor compliance over ergonomics]",
    "    </examples>",
    "</type>",
    "<type>",
    "    <name>reference</name>",
    "    <description>Stores pointers to where information can be found in external systems. These memories allow you to remember where to look to find up-to-date information outside of the project directory.</description>",
    "    <when_to_save>When you learn about resources in external systems and their purpose. For example, that bugs are tracked in a specific project in Linear or that feedback can be found in a specific Slack channel.</when_to_save>",
    "    <how_to_use>When the user references an external system or information that may be in an external system.</how_to_use>",
    "    <examples>",
    "    user: check the Linear project \"INGEST\" if you want context on these tickets, that's where we track all pipeline bugs",
    "    assistant: [saves reference memory: pipeline bugs are tracked in Linear project \"INGEST\"]",
    "",
    "    user: the Grafana board at grafana.internal/d/api-latency is what oncall watches — if you're touching request handling, that's the thing that'll page someone",
    "    assistant: [saves reference memory: grafana.internal/d/api-latency is the oncall latency dashboard — check it when editing request-path code]",
    "    </examples>",
    "</type>",
    "</types>",
    "",
];

pub const WHAT_NOT_TO_SAVE_SECTION: &[&str] = &[
    "## What NOT to save in memory",
    "",
    "- Code patterns, conventions, architecture, file paths, or project structure — these can be derived by reading the current project state.",
    "- Git history, recent changes, or who-changed-what — `git log` / `git blame` are authoritative.",
    "- Debugging solutions or fix recipes — the fix is in the code; the commit message has the context.",
    "- Anything already documented in CLAUDE.md files.",
    "- Ephemeral task details: in-progress work, temporary state, current conversation context.",
    "",
    "These exclusions apply even when the user explicitly asks you to save. If they ask you to save a PR list or activity summary, ask what was *surprising* or *non-obvious* about it — that is the part worth keeping.",
];

pub const MEMORY_DRIFT_CAVEAT: &str =
    "- Memory records can become stale over time. Use memory as context for what was true at a given point in time. Before answering the user or building assumptions based solely on information in memory records, verify that the memory is still correct and up-to-date by reading the current state of the files or resources. If a recalled memory conflicts with current information, trust what you observe now — and update or remove the stale memory rather than acting on it.";

pub const WHEN_TO_ACCESS_SECTION: &[&str] = &[
    "## When to access memories",
    "- When memories seem relevant, or the user references prior-conversation work.",
    "- You MUST access memory when the user explicitly asks you to check, recall, or remember.",
    "- If the user says to *ignore* or *not use* memory: proceed as if MEMORY.md were empty. Do not apply remembered facts, cite, compare against, or mention memory content.",
    MEMORY_DRIFT_CAVEAT,
];

pub const TRUSTING_RECALL_SECTION: &[&str] = &[
    "## Before recommending from memory",
    "",
    "A memory that names a specific function, file, or flag is a claim that it existed *when the memory was written*. It may have been renamed, removed, or never merged. Before recommending it:",
    "",
    "- If the memory names a file path: check the file exists.",
    "- If the memory names a function or flag: grep for it.",
    "- If the user is about to act on your recommendation (not just asking about history), verify first.",
    "",
    "\"The memory says X exists\" is not the same as \"X exists now.\"",
    "",
    "A memory that summarizes repo state (activity logs, architecture snapshots) is frozen in time. If the user asks about *recent* or *current* state, prefer `git log` or reading the code over recalling the snapshot.",
];

/// Produce the frontmatter example block. The third line lists the four
/// valid `type` values, so regenerate from MEMORY_TYPES rather than hard-
/// coding (matches TS behaviour).
pub fn memory_frontmatter_example() -> Vec<String> {
    let types_list = MEMORY_TYPES
        .iter()
        .map(|t| t.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    vec![
        "```markdown".into(),
        "---".into(),
        "name: {{memory name}}".into(),
        "description: {{one-line description — used to decide relevance in future conversations, so be specific}}".into(),
        format!("type: {{{{{}}}}}", types_list),
        "---".into(),
        "".into(),
        "{{memory content — for feedback/project types, structure as: rule/fact, then **Why:** and **How to apply:** lines}}".into(),
        "```".into(),
    ]
}

/// Build the full memory-prompt lines (individual mode) given a display
/// name + absolute memory directory path. Mirrors TS `buildMemoryLines`
/// without the team-memory / extraGuidelines surface.
pub fn build_memory_lines(display_name: &str, memory_dir: &str, skip_index: bool) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();

    lines.push(format!("# {}", display_name));
    lines.push(String::new());
    lines.push(format!(
        "You have a persistent, file-based memory system at `{}`. \
         This directory already exists — write to it directly with the Write tool \
         (do not run mkdir or check for its existence).",
        memory_dir
    ));
    lines.push(String::new());
    lines.push("You should build up this memory system over time so that future conversations can have a complete picture of who the user is, how they'd like to collaborate with you, what behaviors to avoid or repeat, and the context behind the work the user gives you.".into());
    lines.push(String::new());
    lines.push("If the user explicitly asks you to remember something, save it immediately as whichever type fits best. If they ask you to forget something, find and remove the relevant entry.".into());
    lines.push(String::new());

    for s in TYPES_SECTION_INDIVIDUAL {
        lines.push((*s).to_string());
    }
    for s in WHAT_NOT_TO_SAVE_SECTION {
        lines.push((*s).to_string());
    }
    lines.push(String::new());

    // How-to-save block — shorter when skip_index is true (no MEMORY.md).
    if skip_index {
        lines.push("## How to save memories".into());
        lines.push(String::new());
        lines.push(
            "Write each memory to its own file (e.g., `user_role.md`, `feedback_testing.md`) using this frontmatter format:"
                .into(),
        );
        lines.push(String::new());
        lines.extend(memory_frontmatter_example());
        lines.push(String::new());
        lines.push("- Keep the name, description, and type fields in memory files up-to-date with the content".into());
        lines.push("- Organize memory semantically by topic, not chronologically".into());
        lines.push("- Update or remove memories that turn out to be wrong or outdated".into());
        lines.push("- Do not write duplicate memories. First check if there is an existing memory you can update before writing a new one.".into());
    } else {
        lines.push("## How to save memories".into());
        lines.push(String::new());
        lines.push("Saving a memory is a two-step process:".into());
        lines.push(String::new());
        lines.push("**Step 1** — write the memory to its own file (e.g., `user_role.md`, `feedback_testing.md`) using this frontmatter format:".into());
        lines.push(String::new());
        lines.extend(memory_frontmatter_example());
        lines.push(String::new());
        lines.push(format!(
            "**Step 2** — add a pointer to that file in `{e}`. `{e}` is an index, not a memory — each entry should be one line, under ~150 characters: `- [Title](file.md) — one-line hook`. It has no frontmatter. Never write memory content directly into `{e}`.",
            e = ENTRYPOINT_NAME
        ));
        lines.push(String::new());
        lines.push(format!(
            "- `{}` is always loaded into your conversation context — lines after {} will be truncated, so keep the index concise",
            ENTRYPOINT_NAME, MAX_ENTRYPOINT_LINES
        ));
        lines.push("- Keep the name, description, and type fields in memory files up-to-date with the content".into());
        lines.push("- Organize memory semantically by topic, not chronologically".into());
        lines.push("- Update or remove memories that turn out to be wrong or outdated".into());
        lines.push("- Do not write duplicate memories. First check if there is an existing memory you can update before writing a new one.".into());
    }

    lines.push(String::new());
    for s in WHEN_TO_ACCESS_SECTION {
        lines.push((*s).to_string());
    }
    lines.push(String::new());
    for s in TRUSTING_RECALL_SECTION {
        lines.push((*s).to_string());
    }
    lines.push(String::new());

    lines.push("## Memory and other forms of persistence".into());
    lines.push("Memory is one of several persistence mechanisms available to you as you assist the user in a given conversation. The distinction is often that memory can be recalled in future conversations and should not be used for persisting information that is only useful within the scope of the current conversation.".into());
    lines.push("- When to use or update a plan instead of memory: If you are about to start a non-trivial implementation task and would like to reach alignment with the user on your approach you should use a Plan rather than saving this information to memory. Similarly, if you already have a plan within the conversation and you have changed your approach persist that change by updating the plan rather than saving a memory.".into());
    lines.push("- When to use or update tasks instead of memory: When you need to break your work in current conversation into discrete steps or keep track of your progress use tasks instead of saving to memory. Tasks are great for persisting information about the work that needs to be done in the current conversation, but memory should be reserved for information that will be useful in future conversations.".into());
    lines.push(String::new());

    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_lists_all_memory_types() {
        let lines = memory_frontmatter_example();
        let type_line = lines
            .iter()
            .find(|l| l.starts_with("type:"))
            .expect("type line");
        for t in MEMORY_TYPES {
            assert!(
                type_line.contains(t.as_str()),
                "type line {type_line:?} missing {}",
                t.as_str()
            );
        }
    }

    #[test]
    fn build_memory_lines_contains_major_sections() {
        let out = build_memory_lines("auto memory", "/tmp/mem", false);
        let joined = out.join("\n");
        assert!(joined.contains("## Types of memory"));
        assert!(joined.contains("## What NOT to save in memory"));
        assert!(joined.contains("## How to save memories"));
        assert!(joined.contains("## When to access memories"));
        assert!(joined.contains("## Before recommending from memory"));
        assert!(joined.contains("## Memory and other forms of persistence"));
        assert!(joined.contains("/tmp/mem"));
    }

    #[test]
    fn skip_index_removes_step_language() {
        let with_index = build_memory_lines("m", "/tmp/m", false).join("\n");
        let no_index = build_memory_lines("m", "/tmp/m", true).join("\n");
        assert!(with_index.contains("two-step process"));
        assert!(!no_index.contains("two-step process"));
    }

    #[test]
    fn when_to_access_has_drift_caveat() {
        assert!(WHEN_TO_ACCESS_SECTION
            .iter()
            .any(|l| l.contains("become stale over time")));
    }
}
