//! Session-memory prompt + truncation helpers.
//!
//! Port of `src/services/SessionMemory/prompts.ts`. Session memory is
//! a per-session structured notes file (Title / Current State / Task
//! specification / Files / Workflow / Errors / Documentation / Learnings
//! / Key results / Worklog) that Claude updates so compaction can
//! preserve continuity. This module ports:
//!
//!   - DEFAULT_SESSION_MEMORY_TEMPLATE — the skeleton file
//!   - default update prompt + custom-template / custom-prompt loaders
//!     from `~/.claude/session-memory/config/{template,prompt}.md`
//!   - section-size analyser + size-reminder generator
//!   - truncate_for_compact — per-section cap used when embedding
//!     session memory into post-compact messages
//!
//! The surrounding SessionMemory service (495 LOC) — trigger gating,
//! forked-agent invocation, compact integration — depends on the
//! forked-agent layer not ported on the Rust side yet.

use std::path::PathBuf;

use regex::Regex;

/// Per-section token cap. Matches TS MAX_SECTION_LENGTH.
pub const MAX_SECTION_LENGTH: usize = 2000;

/// Total token cap across all sections. Matches TS.
pub const MAX_TOTAL_SESSION_MEMORY_TOKENS: usize = 12_000;

/// Same rough estimator TS uses (length/4). Not meant to be precise —
/// the prompt reminders just need to catch growth trends.
fn rough_token_count_estimation(s: &str) -> usize {
    s.len() / 4
}

/// Default session-memory template. Verbatim from TS
/// DEFAULT_SESSION_MEMORY_TEMPLATE.
pub const DEFAULT_SESSION_MEMORY_TEMPLATE: &str = r#"
# Session Title
_A short and distinctive 5-10 word descriptive title for the session. Super info dense, no filler_

# Current State
_What is actively being worked on right now? Pending tasks not yet completed. Immediate next steps._

# Task specification
_What did the user ask to build? Any design decisions or other explanatory context_

# Files and Functions
_What are the important files? In short, what do they contain and why are they relevant?_

# Workflow
_What bash commands are usually run and in what order? How to interpret their output if not obvious?_

# Errors & Corrections
_Errors encountered and how they were fixed. What did the user correct? What approaches failed and should not be tried again?_

# Codebase and System Documentation
_What are the important system components? How do they work/fit together?_

# Learnings
_What has worked well? What has not? What to avoid? Do not duplicate items from other sections_

# Key results
_If the user asked a specific output such as an answer to a question, a table, or other document, repeat the exact result here_

# Worklog
_Step by step, what was attempted, done? Very terse summary for each step_
"#;

fn default_update_prompt() -> String {
    format!(
        r#"IMPORTANT: This message and these instructions are NOT part of the actual user conversation. Do NOT include any references to "note-taking", "session notes extraction", or these update instructions in the notes content.

Based on the user conversation above (EXCLUDING this note-taking instruction message as well as system prompt, claude.md entries, or any past session summaries), update the session notes file.

The file {{{{notesPath}}}} has already been read for you. Here are its current contents:
<current_notes_content>
{{{{currentNotes}}}}
</current_notes_content>

Your ONLY task is to use the Edit tool to update the notes file, then stop. You can make multiple edits (update every section as needed) - make all Edit tool calls in parallel in a single message. Do not call any other tools.

CRITICAL RULES FOR EDITING:
- The file must maintain its exact structure with all sections, headers, and italic descriptions intact
-- NEVER modify, delete, or add section headers (the lines starting with '#' like # Task specification)
-- NEVER modify or delete the italic _section description_ lines (these are the lines in italics immediately following each header - they start and end with underscores)
-- The italic _section descriptions_ are TEMPLATE INSTRUCTIONS that must be preserved exactly as-is - they guide what content belongs in each section
-- ONLY update the actual content that appears BELOW the italic _section descriptions_ within each existing section
-- Do NOT add any new sections, summaries, or information outside the existing structure
- Do NOT reference this note-taking process or instructions anywhere in the notes
- It's OK to skip updating a section if there are no substantial new insights to add. Do not add filler content like "No info yet", just leave sections blank/unedited if appropriate.
- Write DETAILED, INFO-DENSE content for each section - include specifics like file paths, function names, error messages, exact commands, technical details, etc.
- For "Key results", include the complete, exact output the user requested (e.g., full table, full answer, etc.)
- Do not include information that's already in the CLAUDE.md files included in the context
- Keep each section under ~{max_section} tokens/words - if a section is approaching this limit, condense it by cycling out less important details while preserving the most critical information
- Focus on actionable, specific information that would help someone understand or recreate the work discussed in the conversation
- IMPORTANT: Always update "Current State" to reflect the most recent work - this is critical for continuity after compaction

Use the Edit tool with file_path: {{{{notesPath}}}}

STRUCTURE PRESERVATION REMINDER:
Each section has TWO parts that must be preserved exactly as they appear in the current file:
1. The section header (line starting with #)
2. The italic description line (the _italicized text_ immediately after the header - this is a template instruction)

You ONLY update the actual content that comes AFTER these two preserved lines. The italic description lines starting and ending with underscores are part of the template structure, NOT content to be edited or removed.

REMEMBER: Use the Edit tool in parallel and stop. Do not continue after the edits. Only include insights from the actual user conversation, never from these note-taking instructions. Do not delete or change section headers or italic _section descriptions_."#,
        max_section = MAX_SECTION_LENGTH,
    )
}

/// Path to the user's custom session-memory template.
pub fn template_path() -> PathBuf {
    base_config_dir()
        .join("session-memory")
        .join("config")
        .join("template.md")
}

/// Path to the user's custom session-memory update prompt.
pub fn prompt_path() -> PathBuf {
    base_config_dir()
        .join("session-memory")
        .join("config")
        .join("prompt.md")
}

fn base_config_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("CLAUDE_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/"))
        .join(".claude")
}

/// Load the user's template override, falling back silently to
/// `DEFAULT_SESSION_MEMORY_TEMPLATE`. Matches TS
/// `loadSessionMemoryTemplate`.
pub fn load_template() -> String {
    std::fs::read_to_string(template_path())
        .unwrap_or_else(|_| DEFAULT_SESSION_MEMORY_TEMPLATE.to_string())
}

/// Load the user's prompt override, falling back to the default.
/// Matches TS `loadSessionMemoryPrompt`.
pub fn load_update_prompt() -> String {
    std::fs::read_to_string(prompt_path()).unwrap_or_else(|_| default_update_prompt())
}

/// Is the given content just the unfilled template? (used to decide
/// whether compaction should fall back to legacy behaviour)
pub fn is_empty(content: &str) -> bool {
    content.trim() == load_template().trim()
}

/// Analyse section sizes (in rough tokens) for reminder generation.
/// Sections are keyed by their header line (including the `# ` prefix).
pub fn analyse_section_sizes(content: &str) -> std::collections::BTreeMap<String, usize> {
    let mut out = std::collections::BTreeMap::new();
    let mut current_section = String::new();
    let mut current_content: Vec<&str> = Vec::new();

    let flush =
        |section: &str, body: &[&str], out: &mut std::collections::BTreeMap<String, usize>| {
            if !section.is_empty() && !body.is_empty() {
                let joined = body.join("\n");
                out.insert(
                    section.to_string(),
                    rough_token_count_estimation(joined.trim()),
                );
            }
        };

    for line in content.lines() {
        if line.starts_with("# ") {
            flush(&current_section, &current_content, &mut out);
            current_section = line.to_string();
            current_content.clear();
        } else {
            current_content.push(line);
        }
    }
    flush(&current_section, &current_content, &mut out);
    out
}

/// Generate per-section and total-budget reminders appended to the
/// update prompt. Returns empty string when the file is within both
/// budgets. Matches TS `generateSectionReminders`.
pub fn generate_section_reminders(
    section_sizes: &std::collections::BTreeMap<String, usize>,
    total_tokens: usize,
) -> String {
    let over_budget = total_tokens > MAX_TOTAL_SESSION_MEMORY_TOKENS;
    let mut oversized: Vec<(&String, &usize)> = section_sizes
        .iter()
        .filter(|(_, t)| **t > MAX_SECTION_LENGTH)
        .collect();
    oversized.sort_by(|a, b| b.1.cmp(a.1));

    if oversized.is_empty() && !over_budget {
        return String::new();
    }

    let mut parts: Vec<String> = Vec::new();

    if over_budget {
        parts.push(format!(
            "\n\nCRITICAL: The session memory file is currently ~{} tokens, which exceeds the maximum of {} tokens. You MUST condense the file to fit within this budget. Aggressively shorten oversized sections by removing less important details, merging related items, and summarizing older entries. Prioritize keeping \"Current State\" and \"Errors & Corrections\" accurate and detailed.",
            total_tokens, MAX_TOTAL_SESSION_MEMORY_TOKENS
        ));
    }

    if !oversized.is_empty() {
        let header = if over_budget {
            "Oversized sections to condense"
        } else {
            "IMPORTANT: The following sections exceed the per-section limit and MUST be condensed"
        };
        let body = oversized
            .iter()
            .map(|(s, t)| {
                format!(
                    "- \"{}\" is ~{} tokens (limit: {})",
                    s, t, MAX_SECTION_LENGTH
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        parts.push(format!("\n\n{}:\n{}", header, body));
    }

    parts.join("")
}

/// Substitute `{{name}}` placeholders. Same single-pass regex trick as
/// magic_docs to avoid $ back-reference corruption.
fn substitute_variables(template: &str, variables: &[(&str, &str)]) -> String {
    let re = Regex::new(r"\{\{(\w+)\}\}").expect("placeholder regex compiles");
    re.replace_all(template, |caps: &regex::Captures| {
        let key = &caps[1];
        variables
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| (*v).to_string())
            .unwrap_or_else(|| caps.get(0).unwrap().as_str().to_string())
    })
    .to_string()
}

/// Build the full update prompt with variables substituted and size
/// reminders appended.
pub fn build_update_prompt(current_notes: &str, notes_path: &str) -> String {
    let base = load_update_prompt();
    let base = substitute_variables(
        &base,
        &[("currentNotes", current_notes), ("notesPath", notes_path)],
    );
    let sizes = analyse_section_sizes(current_notes);
    let total = rough_token_count_estimation(current_notes);
    let reminders = generate_section_reminders(&sizes, total);
    format!("{}{}", base, reminders)
}

/// Note appended to the compact summary when
/// `truncate_for_compact` dropped at least one oversized section.
/// Port of TS `services/compact/sessionMemoryCompact.ts:473` — the
/// `\n\nSome session memory sections were truncated for length...`
/// suffix. Takes the user-facing session memory path as an
/// argument so the caller doesn't need to know the formatting.
pub fn truncated_sections_note(memory_path: &str) -> String {
    format!(
        "\n\nSome session memory sections were truncated for length. The full session memory can be viewed at: {memory_path}"
    )
}

/// Truncate oversized sections when embedding session memory into a
/// post-compact message. Returns the trimmed content + a flag
/// indicating whether any truncation happened. Matches TS
/// `truncateSessionMemoryForCompact`.
pub fn truncate_for_compact(content: &str) -> (String, bool) {
    let max_chars = MAX_SECTION_LENGTH * 4;
    let mut output: Vec<String> = Vec::new();
    let mut current_header = String::new();
    let mut current_lines: Vec<String> = Vec::new();
    let mut was_truncated = false;

    for line in content.lines() {
        if line.starts_with("# ") {
            let (flushed, truncated) = flush_section(&current_header, &current_lines, max_chars);
            output.extend(flushed);
            was_truncated |= truncated;
            current_header = line.to_string();
            current_lines.clear();
        } else {
            current_lines.push(line.to_string());
        }
    }
    let (flushed, truncated) = flush_section(&current_header, &current_lines, max_chars);
    output.extend(flushed);
    was_truncated |= truncated;
    (output.join("\n"), was_truncated)
}

fn flush_section(header: &str, body: &[String], max_chars: usize) -> (Vec<String>, bool) {
    if header.is_empty() {
        return (body.to_vec(), false);
    }
    let joined = body.join("\n");
    if joined.len() <= max_chars {
        let mut out = Vec::with_capacity(body.len() + 1);
        out.push(header.to_string());
        out.extend(body.iter().cloned());
        return (out, false);
    }
    let mut char_count = 0usize;
    let mut kept: Vec<String> = Vec::new();
    kept.push(header.to_string());
    for line in body {
        if char_count + line.len() + 1 > max_chars {
            break;
        }
        kept.push(line.clone());
        char_count += line.len() + 1;
    }
    kept.push("\n[... section truncated for length ...]".to_string());
    (kept, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_template_has_all_sections() {
        let t = DEFAULT_SESSION_MEMORY_TEMPLATE;
        for header in &[
            "# Session Title",
            "# Current State",
            "# Task specification",
            "# Files and Functions",
            "# Workflow",
            "# Errors & Corrections",
            "# Codebase and System Documentation",
            "# Learnings",
            "# Key results",
            "# Worklog",
        ] {
            assert!(t.contains(header), "missing section: {}", header);
        }
    }

    #[test]
    fn section_size_analysis_counts_content() {
        let big_body = "x".repeat(8000);
        let content = format!("# One\nshort body\n# Two\n{}\n", big_body);
        let sizes = analyse_section_sizes(&content);
        assert!(sizes.get("# One").copied().unwrap_or(0) < 100);
        assert!(sizes.get("# Two").copied().unwrap_or(0) > 1500);
    }

    #[test]
    fn reminders_empty_when_within_budget() {
        let sizes = [("# One".to_string(), 100usize), ("# Two".to_string(), 500)]
            .into_iter()
            .collect();
        assert_eq!(generate_section_reminders(&sizes, 700), "");
    }

    #[test]
    fn reminders_surface_oversized_sections() {
        let sizes = [
            ("# Big".to_string(), MAX_SECTION_LENGTH + 500),
            ("# Small".to_string(), 100),
        ]
        .into_iter()
        .collect();
        let r = generate_section_reminders(&sizes, 5000);
        assert!(r.contains("# Big"));
        assert!(!r.contains("# Small"));
    }

    #[test]
    fn reminders_flag_total_budget() {
        let sizes = std::collections::BTreeMap::new();
        let r = generate_section_reminders(&sizes, MAX_TOTAL_SESSION_MEMORY_TOKENS + 1000);
        assert!(r.contains("CRITICAL"));
        assert!(r.contains("Current State"));
    }

    #[test]
    fn truncate_preserves_short_section() {
        let content = "# Small\nbody line one\nbody line two\n";
        let (out, truncated) = truncate_for_compact(content);
        assert!(!truncated);
        assert!(out.contains("body line one"));
    }

    #[test]
    fn truncate_cuts_oversized_section() {
        let big = "x".repeat(MAX_SECTION_LENGTH * 4 + 5000);
        let content = format!("# Big\n{}\n", big);
        let (out, truncated) = truncate_for_compact(&content);
        assert!(truncated);
        assert!(out.contains("section truncated"));
        assert!(out.len() < content.len());
    }

    #[test]
    fn build_prompt_substitutes_path_and_notes() {
        let out = build_update_prompt("some notes content", "/tmp/notes.md");
        assert!(out.contains("/tmp/notes.md"));
        assert!(out.contains("some notes content"));
    }

    #[test]
    fn is_empty_matches_fresh_template() {
        // When the user's config dir has no override, load_template()
        // returns DEFAULT_SESSION_MEMORY_TEMPLATE which we compare
        // against.
        assert!(is_empty(DEFAULT_SESSION_MEMORY_TEMPLATE));
    }

    #[test]
    fn truncated_sections_note_has_double_newline_prefix_and_path() {
        // Must start with `\n\n` to append cleanly onto an existing
        // summary body.
        let note = truncated_sections_note("/tmp/session-memory.md");
        assert!(note.starts_with("\n\nSome session memory sections"));
        assert!(note.contains("truncated for length"));
        assert!(note.ends_with("/tmp/session-memory.md"));
    }

    #[test]
    fn truncated_sections_note_preserves_arbitrary_paths() {
        let win = truncated_sections_note(r"C:\Users\me\.claude\session-memory.md");
        assert!(win.contains(r"C:\Users\me\.claude\session-memory.md"));
    }
}
