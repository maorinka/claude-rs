//! Magic Docs update-prompt builder.
//!
//! Port of `src/services/MagicDocs/prompts.ts`. "Magic Docs" are
//! project-owned overview documents that Claude keeps up-to-date by
//! editing them after relevant conversations. This module ships the
//! prompt template + variable-substitution helper + the user-override
//! loader (`~/.claude/magic-docs/prompt.md`).
//!
//! The full MagicDocs service (254 LOC) — trigger heuristics, diff
//! gate, persistence — depends on the forked-agent layer and isn't
//! ported yet.

use std::path::PathBuf;

use regex::Regex;

/// Default update-prompt template. Verbatim from TS
/// `getUpdatePromptTemplate` so prompt-cache prefix-matches when both
/// implementations hit the API with the same template.
pub const DEFAULT_UPDATE_PROMPT_TEMPLATE: &str = "IMPORTANT: This message and these instructions are NOT part of the actual user conversation. Do NOT include any references to \"documentation updates\", \"magic docs\", or these update instructions in the document content.\n\n\
Based on the user conversation above (EXCLUDING this documentation update instruction message), update the Magic Doc file to incorporate any NEW learnings, insights, or information that would be valuable to preserve.\n\n\
The file {{docPath}} has already been read for you. Here are its current contents:\n\
<current_doc_content>\n\
{{docContents}}\n\
</current_doc_content>\n\n\
Document title: {{docTitle}}\n\
{{customInstructions}}\n\n\
Your ONLY task is to use the Edit tool to update the documentation file if there is substantial new information to add, then stop. You can make multiple edits (update multiple sections as needed) - make all Edit tool calls in parallel in a single message. If there's nothing substantial to add, simply respond with a brief explanation and do not call any tools.\n\n\
CRITICAL RULES FOR EDITING:\n\
- Preserve the Magic Doc header exactly as-is: # MAGIC DOC: {{docTitle}}\n\
- If there's an italicized line immediately after the header, preserve it exactly as-is\n\
- Keep the document CURRENT with the latest state of the codebase - this is NOT a changelog or history\n\
- Update information IN-PLACE to reflect the current state - do NOT append historical notes or track changes over time\n\
- Remove or replace outdated information rather than adding \"Previously...\" or \"Updated to...\" notes\n\
- Clean up or DELETE sections that are no longer relevant or don't align with the document's purpose\n\
- Fix obvious errors: typos, grammar mistakes, broken formatting, incorrect information, or confusing statements\n\
- Keep the document well organized: use clear headings, logical section order, consistent formatting, and proper nesting\n\n\
DOCUMENTATION PHILOSOPHY - READ CAREFULLY:\n\
- BE TERSE. High signal only. No filler words or unnecessary elaboration.\n\
- Documentation is for OVERVIEWS, ARCHITECTURE, and ENTRY POINTS - not detailed code walkthroughs\n\
- Do NOT duplicate information that's already obvious from reading the source code\n\
- Do NOT document every function, parameter, or line number reference\n\
- Focus on: WHY things exist, HOW components connect, WHERE to start reading, WHAT patterns are used\n\
- Skip: detailed implementation steps, exhaustive API docs, play-by-play narratives\n\n\
What TO document:\n\
- High-level architecture and system design\n\
- Non-obvious patterns, conventions, or gotchas\n\
- Key entry points and where to start reading code\n\
- Important design decisions and their rationale\n\
- Critical dependencies or integration points\n\
- References to related files, docs, or code (like a wiki) - help readers navigate to relevant context\n\n\
What NOT to document:\n\
- Anything obvious from reading the code itself\n\
- Exhaustive lists of files, functions, or parameters\n\
- Step-by-step implementation details\n\
- Low-level code mechanics\n\
- Information already in CLAUDE.md or other project docs\n\n\
Use the Edit tool with file_path: {{docPath}}\n\n\
REMEMBER: Only update if there is substantial new information. The Magic Doc header (# MAGIC DOC: {{docTitle}}) must remain unchanged.";

/// Path to the user's custom prompt override, matching TS
/// `~/.claude/magic-docs/prompt.md`. Falls through
/// `CLAUDE_CONFIG_DIR` when set.
pub fn custom_prompt_path() -> PathBuf {
    if let Ok(dir) = std::env::var("CLAUDE_CONFIG_DIR") {
        return PathBuf::from(dir).join("magic-docs").join("prompt.md");
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/"))
        .join(".claude")
        .join("magic-docs")
        .join("prompt.md")
}

/// Load the user's custom template from disk, falling back silently
/// to the default when missing / unreadable. Matches TS
/// `loadMagicDocsPrompt`.
pub fn load_update_prompt_template() -> String {
    let path = custom_prompt_path();
    std::fs::read_to_string(&path).unwrap_or_else(|_| DEFAULT_UPDATE_PROMPT_TEMPLATE.to_string())
}

/// Substitute `{{name}}` placeholders from `variables`. Keys not
/// present in the map are left intact. Single-pass regex replacement
/// mirrors the TS implementation (avoids $ back-reference corruption
/// and double-substitution).
pub fn substitute_variables(template: &str, variables: &[(&str, &str)]) -> String {
    let re = Regex::new(r"\{\{(\w+)\}\}").expect("placeholder regex compiles");
    re.replace_all(template, |caps: &regex::Captures| {
        let key = &caps[1];
        match variables.iter().find(|(k, _)| *k == key) {
            Some((_, v)) => (*v).to_string(),
            None => caps.get(0).unwrap().as_str().to_string(),
        }
    })
    .to_string()
}

/// Build the full Magic Docs update prompt with substituted variables.
/// Mirrors TS `buildMagicDocsUpdatePrompt`.
pub fn build_magic_docs_update_prompt(
    doc_contents: &str,
    doc_path: &str,
    doc_title: &str,
    instructions: Option<&str>,
) -> String {
    let template = load_update_prompt_template();
    let custom_instructions = match instructions {
        Some(t) if !t.is_empty() => format!(
            "\n\nDOCUMENT-SPECIFIC UPDATE INSTRUCTIONS:\nThe document author has provided specific instructions for how this file should be updated. Pay extra attention to these instructions and follow them carefully:\n\n\"{}\"\n\nThese instructions take priority over the general rules below. Make sure your updates align with these specific guidelines.",
            t
        ),
        _ => String::new(),
    };
    substitute_variables(
        &template,
        &[
            ("docContents", doc_contents),
            ("docPath", doc_path),
            ("docTitle", doc_title),
            ("customInstructions", custom_instructions.as_str()),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitutes_placeholders() {
        let t = "Hello {{name}}, welcome to {{place}}!";
        let out = substitute_variables(t, &[("name", "world"), ("place", "Rust")]);
        assert_eq!(out, "Hello world, welcome to Rust!");
    }

    #[test]
    fn leaves_unknown_placeholders_intact() {
        let t = "Hello {{name}}, {{unknown}}!";
        let out = substitute_variables(t, &[("name", "x")]);
        assert_eq!(out, "Hello x, {{unknown}}!");
    }

    #[test]
    fn build_prompt_substitutes_all_known_variables() {
        let p = build_magic_docs_update_prompt("# current body", "/tmp/doc.md", "My Doc", None);
        assert!(p.contains("/tmp/doc.md"));
        assert!(p.contains("# current body"));
        assert!(p.contains("# MAGIC DOC: My Doc"));
        assert!(!p.contains("{{docPath}}"));
    }

    #[test]
    fn custom_instructions_appended_when_provided() {
        let p = build_magic_docs_update_prompt(
            "body",
            "/tmp/d.md",
            "D",
            Some("match the existing tone"),
        );
        assert!(p.contains("DOCUMENT-SPECIFIC UPDATE INSTRUCTIONS"));
        assert!(p.contains("match the existing tone"));
    }

    #[test]
    fn custom_instructions_omitted_when_none() {
        let p = build_magic_docs_update_prompt("body", "/tmp/d.md", "D", None);
        assert!(!p.contains("DOCUMENT-SPECIFIC UPDATE INSTRUCTIONS"));
    }

    #[test]
    fn default_template_has_philosophy_markers() {
        assert!(DEFAULT_UPDATE_PROMPT_TEMPLATE.contains("BE TERSE"));
        assert!(DEFAULT_UPDATE_PROMPT_TEMPLATE.contains("DOCUMENTATION PHILOSOPHY"));
    }

    #[test]
    fn custom_prompt_path_honours_env_dir() {
        std::env::set_var("CLAUDE_CONFIG_DIR", "/tmp/mymagicdocs");
        let p = custom_prompt_path();
        assert!(p.starts_with("/tmp/mymagicdocs"));
        assert!(p.ends_with("magic-docs/prompt.md"));
        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }
}
