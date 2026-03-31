use std::path::{Path, PathBuf};
use anyhow::Result;
use serde_json::{json, Value};

use super::git::get_git_context;
use super::environment::build_environment_context;

/// Instruction prefix added before CLAUDE.md contents in the system prompt.
const MEMORY_INSTRUCTION_PROMPT: &str =
    "Codebase and user instructions are shown below. Be sure to adhere to these instructions. \
     IMPORTANT: These instructions OVERRIDE any default behavior and you MUST follow them exactly as written.";

pub async fn build_system_prompt(
    project_root: &Path,
    tool_descriptions: &[(String, String)],  // (name, description)
) -> Result<Vec<Value>> {
    let mut parts: Vec<String> = Vec::new();

    // 1. Base system prompt
    parts.push(base_system_prompt());

    // 2. Tool descriptions
    if !tool_descriptions.is_empty() {
        let mut tools_text = String::from("# Available Tools\n\n");
        for (name, desc) in tool_descriptions {
            tools_text.push_str(&format!("## {}\n{}\n\n", name, desc));
        }
        parts.push(tools_text);
    }

    // 3. Git context
    if let Ok(Some(git_ctx)) = get_git_context(project_root).await {
        parts.push(format!("# Git Context\n{}", git_ctx));
    }

    // 4. Environment
    parts.push(build_environment_context());

    // 5. CLAUDE.md files from parent directories, user home, and project root
    let claude_md_contents = load_claude_md_files(project_root);
    if !claude_md_contents.is_empty() {
        parts.push(MEMORY_INSTRUCTION_PROMPT.to_string());
        for (source, content) in &claude_md_contents {
            parts.push(format!("# Instructions from {}\n\n{}", source, content));
        }
    }

    // Assemble into content blocks
    let blocks: Vec<Value> = parts.into_iter()
        .map(|text| json!({"type": "text", "text": text}))
        .collect();

    Ok(blocks)
}

fn base_system_prompt() -> String {
    "You are Claude, an AI assistant made by Anthropic. You are helping the user with \
     software engineering tasks in their codebase. You have access to tools for reading files, \
     writing files, editing files, searching code, running shell commands, and more. \
     Use these tools to help the user accomplish their goals. Be concise and direct.".to_string()
}

/// Load CLAUDE.md files following the discovery order from the TS implementation:
///
/// 1. User-level: `~/.claude/CLAUDE.md`
/// 2. Parent directories: walk up from project root to filesystem root,
///    loading `CLAUDE.md` and `.claude/CLAUDE.md` from each directory.
///    Files closer to the project root are loaded later (higher priority).
/// 3. Project root: `CLAUDE.md`, `.claude/CLAUDE.md`, `.claude/rules/*.md`
/// 4. Local: `CLAUDE.local.md` in project root
///
/// Returns a list of `(source_label, content)` pairs in priority order
/// (lowest priority first, highest last).
pub fn load_claude_md_files(project_root: &Path) -> Vec<(String, String)> {
    let mut results: Vec<(String, String)> = Vec::new();

    // 1. User-level CLAUDE.md (~/.claude/CLAUDE.md)
    if let Some(home) = dirs::home_dir() {
        let user_claude_md = home.join(".claude").join("CLAUDE.md");
        if let Ok(content) = std::fs::read_to_string(&user_claude_md) {
            if !content.trim().is_empty() {
                results.push(("~/.claude/CLAUDE.md".to_string(), content));
            }
        }
    }

    // 2. Collect parent directories between filesystem root and project root
    //    (excluding the project root itself, which is handled in step 3).
    let mut parent_dirs: Vec<PathBuf> = Vec::new();
    {
        let mut current = project_root.parent();
        while let Some(dir) = current {
            parent_dirs.push(dir.to_path_buf());
            current = dir.parent();
        }
    }
    // Reverse so we go from furthest ancestor to closest parent
    // (furthest = lowest priority, closest = higher priority)
    parent_dirs.reverse();

    for dir in &parent_dirs {
        // CLAUDE.md in the directory itself
        let claude_md = dir.join("CLAUDE.md");
        if let Ok(content) = std::fs::read_to_string(&claude_md) {
            if !content.trim().is_empty() {
                results.push((claude_md.display().to_string(), content));
            }
        }
        // .claude/CLAUDE.md in the directory
        let dotclaude_md = dir.join(".claude").join("CLAUDE.md");
        if let Ok(content) = std::fs::read_to_string(&dotclaude_md) {
            if !content.trim().is_empty() {
                results.push((dotclaude_md.display().to_string(), content));
            }
        }
    }

    // 3. Project root: CLAUDE.md, .claude/CLAUDE.md, .claude/rules/*.md
    let project_claude_md = project_root.join("CLAUDE.md");
    if let Ok(content) = std::fs::read_to_string(&project_claude_md) {
        if !content.trim().is_empty() {
            results.push((project_claude_md.display().to_string(), content));
        }
    }

    let project_dotclaude_md = project_root.join(".claude").join("CLAUDE.md");
    if let Ok(content) = std::fs::read_to_string(&project_dotclaude_md) {
        if !content.trim().is_empty() {
            results.push((project_dotclaude_md.display().to_string(), content));
        }
    }

    // .claude/rules/*.md files
    let rules_dir = project_root.join(".claude").join("rules");
    if let Ok(entries) = std::fs::read_dir(&rules_dir) {
        let mut rule_files: Vec<_> = entries
            .flatten()
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "md")
                    .unwrap_or(false)
            })
            .collect();
        // Sort by filename for deterministic ordering
        rule_files.sort_by_key(|e| e.file_name());
        for entry in rule_files {
            let path = entry.path();
            if let Ok(content) = std::fs::read_to_string(&path) {
                if !content.trim().is_empty() {
                    results.push((path.display().to_string(), content));
                }
            }
        }
    }

    // 4. Local: CLAUDE.local.md in project root
    let local_claude_md = project_root.join("CLAUDE.local.md");
    if let Ok(content) = std::fs::read_to_string(&local_claude_md) {
        if !content.trim().is_empty() {
            results.push((local_claude_md.display().to_string(), content));
        }
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use std::fs;

    #[test]
    fn test_load_claude_md_from_project_root() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(root.join("CLAUDE.md"), "Project instructions").unwrap();

        let results = load_claude_md_files(root);
        assert!(!results.is_empty());
        let project_entry = results.iter().find(|(src, _)| src.contains("CLAUDE.md") && !src.contains(".claude"));
        assert!(project_entry.is_some());
        assert_eq!(project_entry.unwrap().1, "Project instructions");
    }

    #[test]
    fn test_load_claude_md_from_dotclaude_dir() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let dotclaude = root.join(".claude");
        fs::create_dir_all(&dotclaude).unwrap();
        fs::write(dotclaude.join("CLAUDE.md"), "Dotclaude instructions").unwrap();

        let results = load_claude_md_files(root);
        let entry = results.iter().find(|(src, _)| src.contains(".claude/CLAUDE.md"));
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().1, "Dotclaude instructions");
    }

    #[test]
    fn test_load_claude_md_from_rules_dir() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let rules = root.join(".claude").join("rules");
        fs::create_dir_all(&rules).unwrap();
        fs::write(rules.join("style.md"), "Style guide").unwrap();
        fs::write(rules.join("testing.md"), "Testing rules").unwrap();

        let results = load_claude_md_files(root);
        assert!(results.iter().any(|(_, content)| content == "Style guide"));
        assert!(results.iter().any(|(_, content)| content == "Testing rules"));
    }

    #[test]
    fn test_load_claude_md_from_parent_directories() {
        let tmp = TempDir::new().unwrap();
        let parent = tmp.path().join("parent");
        let child = parent.join("child");
        fs::create_dir_all(&child).unwrap();
        fs::write(parent.join("CLAUDE.md"), "Parent instructions").unwrap();
        fs::write(child.join("CLAUDE.md"), "Child instructions").unwrap();

        let results = load_claude_md_files(&child);
        // Parent instructions should come before child instructions (lower priority first)
        let parent_idx = results.iter().position(|(_, c)| c == "Parent instructions");
        let child_idx = results.iter().position(|(_, c)| c == "Child instructions");
        assert!(parent_idx.is_some());
        assert!(child_idx.is_some());
        assert!(parent_idx.unwrap() < child_idx.unwrap());
    }

    #[test]
    fn test_load_claude_md_skips_empty_files() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(root.join("CLAUDE.md"), "  \n  ").unwrap();

        let results = load_claude_md_files(root);
        // Empty/whitespace-only files should be skipped
        assert!(results.iter().all(|(src, _)| !src.ends_with("CLAUDE.md") || !src.contains(root.to_str().unwrap())));
    }

    #[test]
    fn test_load_claude_md_local() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(root.join("CLAUDE.md"), "Project").unwrap();
        fs::write(root.join("CLAUDE.local.md"), "Local overrides").unwrap();

        let results = load_claude_md_files(root);
        // CLAUDE.local.md should be last (highest priority)
        let last = results.last().unwrap();
        assert!(last.0.contains("CLAUDE.local.md"));
        assert_eq!(last.1, "Local overrides");
    }
}
