//! Background memory-extraction agent prompts.
//!
//! Port of `src/services/extractMemories/prompts.ts`. The TS extract
//! agent runs as a forked subprocess that reads the most recent N
//! messages and consolidates them into durable memory files. This
//! module ports the prompt builders; the fork machinery + the trigger
//! flow in extractMemories.ts itself depend on the forked-agent layer
//! that hasn't landed on the Rust side yet.
//!
//! The COMBINED-mode prompt (private + team) relies on a
//! TYPES_SECTION_COMBINED block that isn't ported yet — team memory is
//! still on the deferred list. Callers on the individual (private-only)
//! branch can use this today.

use crate::memdir::prompt::{
    memory_frontmatter_example, TYPES_SECTION_INDIVIDUAL, WHAT_NOT_TO_SAVE_SECTION,
};

/// Tool-name literals used in the prompts. Kept here as constants so
/// the Rust port uses the same names the TS side renders.
const FILE_READ_TOOL_NAME: &str = "Read";
const FILE_EDIT_TOOL_NAME: &str = "Edit";
const FILE_WRITE_TOOL_NAME: &str = "Write";
const GLOB_TOOL_NAME: &str = "Glob";
const GREP_TOOL_NAME: &str = "Grep";
const BASH_TOOL_NAME: &str = "Bash";

/// Shared opener used by both prompt variants. Matches TS `opener`.
///
/// Exposed as [`extract_memories_opener`] for callers that only need
/// the shared preamble (e.g. auditing / snapshot tools).
pub fn extract_memories_opener(new_message_count: u32, existing_memories: &str) -> String {
    opener(new_message_count, existing_memories)
}

fn opener(new_message_count: u32, existing_memories: &str) -> String {
    let manifest = if !existing_memories.is_empty() {
        format!(
            "\n\n## Existing memory files\n\n{}\n\nCheck this list before writing — update an existing file rather than creating a duplicate.",
            existing_memories
        )
    } else {
        String::new()
    };

    format!(
        "You are now acting as the memory extraction subagent. Analyze the most recent ~{n} messages above and use them to update your persistent memory systems.\n\n\
         Available tools: {read}, {grep}, {glob}, read-only {bash} (ls/find/cat/stat/wc/head/tail and similar), and {edit}/{write} for paths inside the memory directory only. {bash} rm is not permitted. All other tools — MCP, Agent, write-capable {bash}, etc — will be denied.\n\n\
         You have a limited turn budget. {edit} requires a prior {read} of the same file, so the efficient strategy is: turn 1 — issue all {read} calls in parallel for every file you might update; turn 2 — issue all {write}/{edit} calls in parallel. Do not interleave reads and writes across multiple turns.\n\n\
         You MUST only use content from the last ~{n} messages to update your persistent memories. Do not waste any turns attempting to investigate or verify that content further — no grepping source files, no reading code to confirm a pattern exists, no git commands.{manifest}",
        n = new_message_count,
        read = FILE_READ_TOOL_NAME,
        edit = FILE_EDIT_TOOL_NAME,
        write = FILE_WRITE_TOOL_NAME,
        glob = GLOB_TOOL_NAME,
        grep = GREP_TOOL_NAME,
        bash = BASH_TOOL_NAME,
        manifest = manifest,
    )
}

fn how_to_save(skip_index: bool) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    out.push("## How to save memories".into());
    out.push(String::new());

    if skip_index {
        out.push(
            "Write each memory to its own file (e.g., `user_role.md`, `feedback_testing.md`) using this frontmatter format:"
                .into(),
        );
        out.push(String::new());
        out.extend(memory_frontmatter_example());
        out.push(String::new());
        out.push("- Organize memory semantically by topic, not chronologically".into());
        out.push("- Update or remove memories that turn out to be wrong or outdated".into());
        out.push(
            "- Do not write duplicate memories. First check if there is an existing memory you can update before writing a new one."
                .into(),
        );
    } else {
        out.push("Saving a memory is a two-step process:".into());
        out.push(String::new());
        out.push(
            "**Step 1** — write the memory to its own file (e.g., `user_role.md`, `feedback_testing.md`) using this frontmatter format:"
                .into(),
        );
        out.push(String::new());
        out.extend(memory_frontmatter_example());
        out.push(String::new());
        out.push(
            "**Step 2** — add a pointer to that file in `MEMORY.md`. `MEMORY.md` is an index, not a memory — each entry should be one line, under ~150 characters: `- [Title](file.md) — one-line hook`. It has no frontmatter. Never write memory content directly into `MEMORY.md`."
                .into(),
        );
        out.push(String::new());
        out.push(
            "- `MEMORY.md` is always loaded into your system prompt — lines after 200 will be truncated, so keep the index concise"
                .into(),
        );
        out.push("- Organize memory semantically by topic, not chronologically".into());
        out.push("- Update or remove memories that turn out to be wrong or outdated".into());
        out.push(
            "- Do not write duplicate memories. First check if there is an existing memory you can update before writing a new one."
                .into(),
        );
    }

    out
}

/// Build the combined auto + team memory prompt. Port of TS
/// `buildExtractCombinedPrompt`.
///
/// TS short-circuits to `buildExtractAutoOnlyPrompt` when the
/// `TEAMMEM` build feature is off. The Rust port does not enable
/// `TEAMMEM` (team-memory TYPES_SECTION_COMBINED isn't ported yet),
/// so this function always falls through to the auto-only variant.
/// Kept as a separate symbol so call sites can opt into the
/// combined pathway once the team-memory prompt block lands,
/// without re-wiring their imports.
pub fn build_extract_combined_prompt(
    new_message_count: u32,
    existing_memories: &str,
    skip_index: bool,
) -> String {
    // Matches TS: `if (!feature('TEAMMEM')) return buildExtractAutoOnlyPrompt(...)`.
    // The TEAMMEM-on branch requires `TYPES_SECTION_COMBINED` +
    // team-scope wording that's on the deferred memdir milestone.
    build_extract_auto_only_prompt(new_message_count, existing_memories, skip_index)
}

/// Build the extract-agent prompt for private-only memory (single
/// directory, no team memory). Verbatim port of TS
/// `buildExtractAutoOnlyPrompt`.
pub fn build_extract_auto_only_prompt(
    new_message_count: u32,
    existing_memories: &str,
    skip_index: bool,
) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push(opener(new_message_count, existing_memories));
    lines.push(String::new());
    lines.push(
        "If the user explicitly asks you to remember something, save it immediately as whichever type fits best. If they ask you to forget something, find and remove the relevant entry."
            .into(),
    );
    lines.push(String::new());
    for s in TYPES_SECTION_INDIVIDUAL {
        lines.push((*s).to_string());
    }
    for s in WHAT_NOT_TO_SAVE_SECTION {
        lines.push((*s).to_string());
    }
    lines.push(String::new());
    lines.extend(how_to_save(skip_index));
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opener_includes_message_count() {
        let p = build_extract_auto_only_prompt(40, "", false);
        assert!(p.contains("the most recent ~40 messages"));
    }

    #[test]
    fn opener_mentions_tool_names() {
        let p = build_extract_auto_only_prompt(10, "", false);
        assert!(p.contains("Read"));
        assert!(p.contains("Grep"));
        assert!(p.contains("Edit"));
        assert!(p.contains("Bash"));
    }

    #[test]
    fn existing_manifest_appended_when_provided() {
        let p = build_extract_auto_only_prompt(10, "- user_role.md", false);
        assert!(p.contains("## Existing memory files"));
        assert!(p.contains("- user_role.md"));
    }

    #[test]
    fn existing_manifest_omitted_when_empty() {
        let p = build_extract_auto_only_prompt(10, "", false);
        assert!(!p.contains("## Existing memory files"));
    }

    #[test]
    fn prompt_contains_all_sections() {
        let p = build_extract_auto_only_prompt(10, "", false);
        assert!(p.contains("## Types of memory"));
        assert!(p.contains("## What NOT to save in memory"));
        assert!(p.contains("## How to save memories"));
    }

    #[test]
    fn skip_index_drops_two_step_language() {
        let without = build_extract_auto_only_prompt(10, "", false);
        let with_skip = build_extract_auto_only_prompt(10, "", true);
        assert!(without.contains("two-step process"));
        assert!(!with_skip.contains("two-step process"));
    }

    #[test]
    fn parallel_read_write_guidance_present() {
        let p = build_extract_auto_only_prompt(10, "", false);
        assert!(p.contains("turn 1"));
        assert!(p.contains("turn 2"));
        assert!(p.contains("in parallel"));
    }

    #[test]
    fn public_opener_matches_private_helper_output() {
        let a = extract_memories_opener(12, "- foo.md");
        let b = opener(12, "- foo.md");
        assert_eq!(a, b);
        assert!(a.contains("~12 messages"));
        assert!(a.contains("## Existing memory files"));
    }

    #[test]
    fn combined_prompt_falls_back_to_auto_only_without_teammem() {
        // Rust port never has TEAMMEM on, so combined must equal
        // auto-only for identical inputs.
        let combined = build_extract_combined_prompt(15, "", false);
        let auto_only = build_extract_auto_only_prompt(15, "", false);
        assert_eq!(combined, auto_only);
    }

    #[test]
    fn combined_prompt_respects_skip_index() {
        let skip = build_extract_combined_prompt(5, "", true);
        assert!(!skip.contains("two-step process"));
        let full = build_extract_combined_prompt(5, "", false);
        assert!(full.contains("two-step process"));
    }
}
