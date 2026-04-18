//! Output styles loader.
//!
//! Ports `src/outputStyles/loadOutputStylesDir.ts`. Reads markdown files from
//! `~/.claude/output-styles/*.md` (user-level) and `.claude/output-styles/*.md`
//! (project-level). Each file is a named style with optional YAML frontmatter:
//! `name`, `description`, `keep-coding-instructions`.
//!
//! Project styles override user styles on name collision, matching TS.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// A single output-style definition.
#[derive(Debug, Clone, PartialEq)]
pub struct OutputStyle {
    pub name: String,
    pub description: String,
    /// The post-frontmatter markdown body, used as the style prompt.
    pub prompt: String,
    /// Where this style was loaded from (directory path).
    pub source: PathBuf,
    /// Optional — whether to keep the default coding-instructions preamble
    /// when this style is active. `None` means defer to the global setting.
    pub keep_coding_instructions: Option<bool>,
}

/// Parse YAML frontmatter at the top of a markdown file.
/// Returns (frontmatter_map, body). Mirrors the bulk of what the TS
/// `loadMarkdownFilesForSubdir` chain produces.
fn parse_frontmatter(src: &str) -> (HashMap<String, String>, String) {
    let mut map = HashMap::new();
    let trimmed = src.trim_start_matches('\u{FEFF}');
    if !trimmed.starts_with("---\n") && !trimmed.starts_with("---\r\n") {
        return (map, src.to_string());
    }

    // Find the closing '---' line
    let after_open = trimmed.splitn(2, '\n').nth(1).unwrap_or("");
    let close_idx = after_open.find("\n---\n")
        .or_else(|| after_open.find("\n---\r\n"))
        .or_else(|| {
            // Handle the case where --- is the last line of the file
            if after_open.ends_with("\n---") {
                Some(after_open.len() - 4)
            } else {
                None
            }
        });

    let Some(close) = close_idx else {
        return (map, src.to_string());
    };

    let yaml_block = &after_open[..close];
    // Body starts after the closing "---\n" (or "---\r\n")
    let rest_start = close + "\n---\n".len();
    let body = if rest_start <= after_open.len() {
        &after_open[rest_start.min(after_open.len())..]
    } else {
        ""
    };

    for line in yaml_block.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim().to_string();
            let val = v.trim().trim_matches(|c| c == '"' || c == '\'').to_string();
            map.insert(key, val);
        }
    }

    (map, body.to_string())
}

fn extract_description_from_markdown(body: &str, fallback: &str) -> String {
    // First non-empty, non-heading line = description. Mirrors TS
    // `extractDescriptionFromMarkdown` behaviour.
    for line in body.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        return t.to_string();
    }
    fallback.to_string()
}

fn load_dir(dir: &Path, out: &mut HashMap<String, OutputStyle>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }
        let Ok(raw) = fs::read_to_string(&path) else {
            continue;
        };
        let (frontmatter, body) = parse_frontmatter(&raw);
        let style_name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();

        let name = frontmatter
            .get("name")
            .cloned()
            .unwrap_or_else(|| style_name.clone());
        let description = frontmatter.get("description").cloned().unwrap_or_else(|| {
            extract_description_from_markdown(&body, &format!("Custom {} output style", style_name))
        });
        let keep_coding_instructions = frontmatter
            .get("keep-coding-instructions")
            .map(|v| v.to_ascii_lowercase())
            .and_then(|v| match v.as_str() {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            });

        out.insert(
            name.clone(),
            OutputStyle {
                name,
                description,
                prompt: body.trim().to_string(),
                source: dir.to_path_buf(),
                keep_coding_instructions,
            },
        );
    }
}

/// Walk `.claude/output-styles` from cwd upward to root, then the user's
/// `~/.claude/output-styles`. Project styles (found first) override user
/// styles on name collision, matching TS semantics where the nearer
/// directory wins.
pub fn load_output_styles(cwd: &Path) -> Vec<OutputStyle> {
    let mut out: HashMap<String, OutputStyle> = HashMap::new();

    // User-level first, so project-level can override.
    if let Some(home) = dirs::home_dir() {
        load_dir(&home.join(".claude").join("output-styles"), &mut out);
    }

    // Walk cwd up to the root, loading each `.claude/output-styles` we find.
    // Inner directories override outer ones because we walk root-to-cwd, so
    // more specific entries are written last.
    let mut ancestors: Vec<&Path> = cwd.ancestors().collect();
    ancestors.reverse();
    for dir in ancestors {
        load_dir(&dir.join(".claude").join("output-styles"), &mut out);
    }

    let mut v: Vec<OutputStyle> = out.into_values().collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn frontmatter_parsed() {
        let src = "---\nname: Terse\ndescription: Short\nkeep-coding-instructions: false\n---\n\nBody line.\n";
        let (fm, body) = parse_frontmatter(src);
        assert_eq!(fm.get("name").map(|s| s.as_str()), Some("Terse"));
        assert_eq!(fm.get("description").map(|s| s.as_str()), Some("Short"));
        assert!(body.trim_start().starts_with("Body line"));
    }

    #[test]
    fn no_frontmatter() {
        let src = "# Heading\n\nBody line.";
        let (fm, body) = parse_frontmatter(src);
        assert!(fm.is_empty());
        assert_eq!(body, src);
    }

    #[test]
    fn loads_project_styles() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join(".claude").join("output-styles");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("terse.md"),
            "---\ndescription: A short style\n---\n\nBe brief.\n",
        )
        .unwrap();

        let styles = load_output_styles(tmp.path());
        let terse = styles.iter().find(|s| s.name == "terse").expect("terse");
        assert_eq!(terse.description, "A short style");
        assert!(terse.prompt.contains("Be brief"));
    }
}
