//! Walker for `.claude/<subdir>/*.md` config files.
//!
//! Port of the useful half of `src/utils/markdownConfigLoader.ts`. TS
//! ships 600 LOC with: canonical git root resolution via ripgrep +
//! realpath, managed-file path computation, plugin-only policy, tool-
//! list frontmatter parsing that delegates to permissionSetup, and
//! three layers of cache for the ripgrep output. This module ships
//! the 1) constant list of recognised subdirs, 2) the description
//! extractor, 3) the frontmatter tool-list parser, and 4) a simpler
//! loader that walks the user home + project root + cwd for .md files
//! under the given subdir.
//!
//! The three-source merge (user / project / managed) still happens;
//! what's different is that we use a straight cwd.ancestors() walk
//! instead of ripgrep-driven canonical-git-root resolution. When
//! getProjectRoot() / findCanonicalGitRoot() get ported, callers can
//! swap in a richer source chain here.

use std::fs;
use std::path::{Path, PathBuf};

use crate::frontmatter::{parse_frontmatter, Frontmatter, FrontmatterValue};

/// Subdirs under `.claude/` that hold user-loadable markdown config.
/// Matches TS CLAUDE_CONFIG_DIRECTORIES (minus the TEMPLATES feature
/// flag — always include templates if you're porting the flag).
pub const CLAUDE_CONFIG_DIRECTORIES: &[&str] = &[
    "commands",
    "agents",
    "output-styles",
    "skills",
    "workflows",
];

/// Where did this markdown file come from? Matches a narrow subset of
/// TS SettingSource — the loader only ever labels user/project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkdownSource {
    /// ~/.claude/<subdir>/...
    User,
    /// <project>/.claude/<subdir>/...
    Project,
}

#[derive(Debug, Clone)]
pub struct MarkdownFile {
    pub file_path: PathBuf,
    pub base_dir: PathBuf,
    pub frontmatter: Frontmatter,
    pub content: String,
    pub source: MarkdownSource,
}

/// Extract a one-line description from markdown content. Strips a
/// leading markdown header. Truncates to 100 chars with a "..." tail.
/// Matches TS extractDescriptionFromMarkdown.
pub fn extract_description_from_markdown(content: &str, default_description: &str) -> String {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let text = if let Some(rest) = trimmed.strip_prefix('#') {
            rest.trim_start_matches('#').trim().to_string()
        } else {
            trimmed.to_string()
        };
        return if text.chars().count() > 100 {
            let mut truncated: String = text.chars().take(97).collect();
            truncated.push_str("...");
            truncated
        } else {
            text
        };
    }
    default_description.to_string()
}

/// Parse a tool-list frontmatter value. Returns:
///   - `None` when the field is absent or null (caller decides the
///     default — agents use "all tools", skills use "no tools").
///   - `Some(vec![])` when the field is empty.
///   - `Some(["*"])` when the list includes a wildcard.
///   - `Some(parsed)` otherwise.
///
/// Matches TS parseToolListString minus the CLI-specific
/// `parseToolListFromCLI` expansion (that parser resolves plugin
/// names + wildcards and isn't ported yet).
pub fn parse_tool_list(value: Option<&FrontmatterValue>) -> Option<Vec<String>> {
    let v = value?;
    match v {
        FrontmatterValue::Null => None,
        FrontmatterValue::String(s) if s.is_empty() => Some(Vec::new()),
        FrontmatterValue::String(s) => {
            let tools: Vec<String> =
                s.split(',').map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect();
            if tools.iter().any(|t| t == "*") {
                Some(vec!["*".into()])
            } else {
                Some(tools)
            }
        }
        FrontmatterValue::List(items) => {
            let mut tools: Vec<String> = Vec::new();
            for item in items {
                if let FrontmatterValue::String(s) = item {
                    if !s.is_empty() {
                        tools.push(s.clone());
                    }
                }
            }
            if tools.iter().any(|t| t == "*") {
                Some(vec!["*".into()])
            } else {
                Some(tools)
            }
        }
        _ => None,
    }
}

/// Load all `.md` files under `.claude/<subdir>/` from both the user
/// home directory (`~/.claude/<subdir>`) and the project root (starting
/// from `cwd` and walking up to the filesystem root, first
/// `.claude/<subdir>` wins). Returns the user files first, then
/// project files — caller applies project-wins-on-name-collision if
/// desired (same semantics as output_styles::load_output_styles).
pub fn load_markdown_files_for_subdir(subdir: &str, cwd: &Path) -> Vec<MarkdownFile> {
    let mut out: Vec<MarkdownFile> = Vec::new();

    if let Some(home) = dirs::home_dir() {
        let dir = home.join(".claude").join(subdir);
        out.extend(load_from_dir(&dir, MarkdownSource::User));
    }

    // Walk cwd up looking for the nearest .claude/<subdir>. We stop at
    // the first match (nearest-wins, matches TS when getProjectRoot
    // resolves to the current cwd's git root).
    for ancestor in cwd.ancestors() {
        let dir = ancestor.join(".claude").join(subdir);
        if dir.is_dir() {
            out.extend(load_from_dir(&dir, MarkdownSource::Project));
            break;
        }
    }

    out
}

fn load_from_dir(dir: &Path, source: MarkdownSource) -> Vec<MarkdownFile> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }
        let Ok(raw) = fs::read_to_string(&path) else {
            continue;
        };
        let parsed = parse_frontmatter(&raw);
        out.push(MarkdownFile {
            file_path: path,
            base_dir: dir.to_path_buf(),
            frontmatter: parsed.frontmatter,
            content: parsed.content,
            source,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn extract_description_first_non_empty() {
        let s = "\n\n  \n# Header Title\nBody line\n";
        assert_eq!(
            extract_description_from_markdown(s, "fallback"),
            "Header Title"
        );
    }

    #[test]
    fn extract_description_uses_fallback_on_empty_input() {
        assert_eq!(
            extract_description_from_markdown("", "default"),
            "default"
        );
    }

    #[test]
    fn extract_description_truncates_long_line() {
        let long = "x".repeat(200);
        let s = format!("{}\n", long);
        let d = extract_description_from_markdown(&s, "f");
        assert!(d.len() <= 100);
        assert!(d.ends_with("..."));
    }

    #[test]
    fn parse_tool_list_absent_is_none() {
        assert_eq!(parse_tool_list(None), None);
        assert_eq!(parse_tool_list(Some(&FrontmatterValue::Null)), None);
    }

    #[test]
    fn parse_tool_list_empty_string_is_empty_vec() {
        assert_eq!(
            parse_tool_list(Some(&FrontmatterValue::String(String::new()))),
            Some(Vec::new())
        );
    }

    #[test]
    fn parse_tool_list_string_splits_on_comma() {
        let v = FrontmatterValue::String("Read, Write, Grep".into());
        assert_eq!(
            parse_tool_list(Some(&v)),
            Some(vec!["Read".into(), "Write".into(), "Grep".into()])
        );
    }

    #[test]
    fn parse_tool_list_wildcard_collapses() {
        let v = FrontmatterValue::String("Read, *, Write".into());
        assert_eq!(parse_tool_list(Some(&v)), Some(vec!["*".into()]));
    }

    #[test]
    fn parse_tool_list_from_list_value() {
        let v = FrontmatterValue::List(vec![
            FrontmatterValue::String("Read".into()),
            FrontmatterValue::String("Write".into()),
        ]);
        assert_eq!(
            parse_tool_list(Some(&v)),
            Some(vec!["Read".into(), "Write".into()])
        );
    }

    #[test]
    fn loads_project_markdown_files() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join(".claude").join("commands");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("test.md"),
            "---\nname: Test\ndescription: A test command\n---\n\nBody\n",
        )
        .unwrap();
        let files = load_markdown_files_for_subdir("commands", tmp.path());
        let project_files: Vec<_> = files
            .into_iter()
            .filter(|f| f.source == MarkdownSource::Project)
            .collect();
        assert_eq!(project_files.len(), 1);
        assert_eq!(
            project_files[0]
                .frontmatter
                .get("name")
                .and_then(|v| v.as_str()),
            Some("Test")
        );
    }

    #[test]
    fn subdir_list_includes_common_dirs() {
        for &d in CLAUDE_CONFIG_DIRECTORIES {
            assert!(!d.is_empty());
        }
        assert!(CLAUDE_CONFIG_DIRECTORIES.contains(&"commands"));
        assert!(CLAUDE_CONFIG_DIRECTORIES.contains(&"skills"));
        assert!(CLAUDE_CONFIG_DIRECTORIES.contains(&"output-styles"));
    }
}
