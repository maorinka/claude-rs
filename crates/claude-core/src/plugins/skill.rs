use std::collections::HashMap;
use std::path::Path;

use crate::config::paths::claude_dir;
use crate::plugins::types::{Skill, SkillSource};

/// YAML frontmatter fields recognised in skill markdown files.
///
/// A skill file is a markdown document with an optional YAML frontmatter
/// block delimited by `---`. The body (everything after the frontmatter)
/// becomes the skill's prompt content.
#[derive(Debug, Clone, Default)]
pub struct SkillFrontmatter {
    pub name: Option<String>,
    pub description: Option<String>,
    pub argument_hint: Option<String>,
    pub when_to_use: Option<String>,
    pub allowed_tools: Vec<String>,
    pub user_invocable: Option<bool>,
    pub disable_model_invocation: Option<bool>,
}

/// Result of parsing a skill markdown file.
#[derive(Debug, Clone)]
pub struct ParsedSkillFile {
    pub frontmatter: SkillFrontmatter,
    pub content: String,
}

// ---------------------------------------------------------------------------
// Frontmatter parsing
// ---------------------------------------------------------------------------

/// Parse a skill markdown file into frontmatter metadata and body content.
///
/// The frontmatter block is delimited by `---` on its own line at the very
/// start of the file. If no frontmatter is present the entire input is
/// treated as the content body and default frontmatter is returned.
///
/// This mirrors the TypeScript `parseFrontmatter` function from
/// `src/utils/frontmatterParser.ts`.
pub fn parse_skill_file(input: &str) -> ParsedSkillFile {
    let (frontmatter, content) = split_frontmatter(input);

    let fm = match frontmatter {
        Some(yaml_text) => parse_yaml_frontmatter(yaml_text),
        None => SkillFrontmatter::default(),
    };

    ParsedSkillFile {
        frontmatter: fm,
        content: content.to_string(),
    }
}

/// Split a markdown document into its optional YAML frontmatter block and
/// the remaining body.
///
/// Returns `(Some(yaml_text), body)` when the document starts with `---`,
/// or `(None, full_input)` otherwise.
fn split_frontmatter(input: &str) -> (Option<&str>, &str) {
    // Must start with `---` followed by optional whitespace and a newline.
    let input_trimmed = input.trim_start_matches('\u{feff}'); // BOM
    if !input_trimmed.starts_with("---") {
        return (None, input);
    }

    // Find the closing `---` delimiter after the opening one.
    let after_opening = &input_trimmed[3..];
    // Skip the rest of the opening line (e.g. `---\n` or `--- \n`).
    let newline_pos = match after_opening.find('\n') {
        Some(pos) => pos,
        None => return (None, input), // single-line `---` with no closing
    };
    let yaml_start = 3 + newline_pos + 1; // offset into input_trimmed

    // Scan for the closing `---` that appears at the start of a line.
    let remaining = &input_trimmed[yaml_start..];
    for (offset, _) in remaining.match_indices("\n---") {
        let after_close = yaml_start + offset + 4; // skip `\n---`
        // The `---` must be followed by optional whitespace then newline or EOF.
        let tail = &input_trimmed[after_close..];
        let rest_start = if tail.is_empty() {
            after_close
        } else if tail.starts_with('\n') {
            after_close + 1
        } else {
            let ws_end = tail.len() - tail.trim_start().len();
            let after_ws = &tail[ws_end..];
            if after_ws.is_empty() || after_ws.starts_with('\n') {
                after_close + ws_end + if after_ws.starts_with('\n') { 1 } else { 0 }
            } else {
                continue; // not a valid closing delimiter
            }
        };

        let yaml_text = &input_trimmed[yaml_start..yaml_start + offset + 1]; // include trailing \n
        // Trim the trailing newline from the yaml text for cleaner parsing
        let yaml_text = yaml_text.trim();
        let content = &input_trimmed[rest_start..];
        return (Some(yaml_text), content);
    }

    // Also check if the file starts with `---\n` and has `---` right at
    // the beginning of the remaining content (no preceding newline needed
    // for the very first line of `remaining`).
    if remaining.starts_with("---") {
        let after_close = yaml_start + 3;
        let tail = &input_trimmed[after_close..];
        let rest_start = if tail.is_empty() {
            after_close
        } else if tail.starts_with('\n') {
            after_close + 1
        } else {
            return (None, input);
        };
        // Empty frontmatter
        return (Some(""), &input_trimmed[rest_start..]);
    }

    (None, input)
}

/// Parse YAML text into a `SkillFrontmatter` struct.
///
/// We intentionally avoid pulling in a full YAML parser crate for this
/// narrow use-case. The frontmatter fields are simple key: value pairs
/// (scalars and lists) so a lightweight line-based parser suffices.
fn parse_yaml_frontmatter(yaml: &str) -> SkillFrontmatter {
    let map = simple_yaml_parse(yaml);
    let mut fm = SkillFrontmatter::default();

    if let Some(v) = map.get("name") {
        fm.name = Some(unquote_yaml_string(v));
    }
    if let Some(v) = map.get("description") {
        fm.description = Some(unquote_yaml_string(v));
    }
    if let Some(v) = map.get("argument-hint") {
        fm.argument_hint = Some(unquote_yaml_string(v));
    }
    if let Some(v) = map.get("when_to_use") {
        fm.when_to_use = Some(unquote_yaml_string(v));
    }
    if let Some(v) = map.get("user-invocable") {
        fm.user_invocable = parse_bool_value(v);
    }
    if let Some(v) = map.get("disable-model-invocation") {
        fm.disable_model_invocation = parse_bool_value(v);
    }

    // `allowed-tools` can be a comma-separated string or a YAML list.
    if let Some(v) = map.get("allowed-tools") {
        fm.allowed_tools = parse_string_list(v);
    }

    fm
}

/// Minimal YAML parser that handles flat key: value pairs.
///
/// Supports:
/// - `key: value`
/// - `key: "quoted value"`
/// - List items (`- item`) as a comma-joined value for the preceding key
fn simple_yaml_parse(yaml: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut current_key: Option<String> = None;
    let mut list_items: Vec<String> = Vec::new();

    for line in yaml.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // List item under the current key
        if trimmed.starts_with("- ") {
            if current_key.is_some() {
                let item = trimmed.strip_prefix("- ").unwrap().trim();
                list_items.push(unquote_yaml_string(item));
            }
            continue;
        }

        // Flush any accumulated list items for the previous key.
        if let Some(key) = current_key.take() {
            if !list_items.is_empty() {
                map.insert(key, list_items.join(", "));
                list_items.clear();
            }
        }

        // key: value
        if let Some((key, value)) = trimmed.split_once(':') {
            let key = key.trim().to_string();
            let value = value.trim();
            if value.is_empty() {
                // Might be followed by list items
                current_key = Some(key);
            } else {
                map.insert(key, value.to_string());
            }
        }
    }

    // Flush trailing list
    if let Some(key) = current_key {
        if !list_items.is_empty() {
            map.insert(key, list_items.join(", "));
        }
    }

    map
}

/// Remove surrounding quotes (single or double) from a YAML scalar.
fn unquote_yaml_string(s: &str) -> String {
    let s = s.trim();
    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

/// Parse a boolean-ish YAML value.
fn parse_bool_value(s: &str) -> Option<bool> {
    match s.trim().to_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        _ => None,
    }
}

/// Parse a value that may be a comma-separated string or was accumulated
/// as a YAML list (already comma-joined by `simple_yaml_parse`).
fn parse_string_list(s: &str) -> Vec<String> {
    s.split(',')
        .map(|item| unquote_yaml_string(item))
        .filter(|item| !item.is_empty())
        .collect()
}

// ---------------------------------------------------------------------------
// Skill discovery
// ---------------------------------------------------------------------------

/// Search locations for skill files.
///
/// Skills live in `.claude/skills/` directories. Each skill is a
/// subdirectory containing a `SKILL.md` file (matching the TS convention in
/// `loadSkillsFromSkillsDir`).
///
/// Discovery order (mirrors TypeScript `getSkillDirCommands`):
/// 1. `~/.claude/skills/` — user-level skills
/// 2. `<project_root>/.claude/skills/` — project-level skills
pub fn discover_skills(project_root: &Path) -> Vec<Skill> {
    let mut skills = Vec::new();
    let mut seen_names = std::collections::HashSet::new();

    // 1. User-level skills (~/.claude/skills/)
    if let Ok(home) = claude_dir() {
        let user_skills_dir = home.join("skills");
        load_skills_from_dir(&user_skills_dir, &SkillSource::Builtin, &mut skills, &mut seen_names);
    }

    // 2. Project-level skills (<project>/.claude/skills/)
    let project_skills_dir = project_root.join(".claude").join("skills");
    let source = SkillSource::Directory(project_skills_dir.clone());
    load_skills_from_dir(&project_skills_dir, &source, &mut skills, &mut seen_names);

    skills
}

/// Load skill directories from a base path.
///
/// Each immediate subdirectory of `base_dir` is expected to contain a
/// `SKILL.md` file. The subdirectory name becomes the skill's name unless
/// overridden by a `name` field in the frontmatter.
fn load_skills_from_dir(
    base_dir: &Path,
    source: &SkillSource,
    skills: &mut Vec<Skill>,
    seen_names: &mut std::collections::HashSet<String>,
) {
    let entries = match std::fs::read_dir(base_dir) {
        Ok(entries) => entries,
        Err(_) => return, // directory doesn't exist or is inaccessible
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let skill_file = path.join("SKILL.md");
        let content = match std::fs::read_to_string(&skill_file) {
            Ok(c) => c,
            Err(_) => continue, // no SKILL.md in this subdirectory
        };

        let dir_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };

        let parsed = parse_skill_file(&content);
        let name = parsed
            .frontmatter
            .name
            .clone()
            .unwrap_or_else(|| dir_name.clone());

        // Deduplicate — first-seen wins (user-level before project-level).
        if !seen_names.insert(name.clone()) {
            continue;
        }

        let description = parsed
            .frontmatter
            .description
            .clone()
            .unwrap_or_else(|| format!("Skill: {}", name));

        skills.push(Skill {
            name,
            description,
            content: parsed.content,
            source: source.clone(),
            argument_hint: parsed.frontmatter.argument_hint,
            when_to_use: parsed.frontmatter.when_to_use,
            allowed_tools: parsed.frontmatter.allowed_tools,
            user_invocable: parsed.frontmatter.user_invocable.unwrap_or(true),
            disable_model_invocation: parsed
                .frontmatter
                .disable_model_invocation
                .unwrap_or(false),
        });
    }
}

/// Check whether user input matches a skill's trigger criteria.
///
/// A match occurs when:
/// 1. The input starts with `/<skill_name>` (explicit invocation), OR
/// 2. The skill has a `when_to_use` hint and the input contains the skill
///    name as a word.
///
/// Returns `Some(args)` with the remaining text after the skill name on
/// explicit match, or `Some("")` on a fuzzy match.
pub fn match_skill<'a>(input: &'a str, skill: &Skill) -> Option<&'a str> {
    let trimmed = input.trim();

    // Explicit slash-command invocation: /skill-name [args...]
    if let Some(rest) = trimmed.strip_prefix('/') {
        if rest == skill.name || rest.starts_with(&format!("{} ", skill.name)) {
            let args = rest
                .strip_prefix(&skill.name)
                .unwrap_or("")
                .trim_start();
            return Some(args);
        }
    }

    // Fuzzy match: skill has a when_to_use hint and the input mentions the
    // skill name as a whole word.
    if skill.when_to_use.is_some() {
        let lower = trimmed.to_lowercase();
        let name_lower = skill.name.to_lowercase();
        // Simple word-boundary check
        if lower.split_whitespace().any(|w| w == name_lower) {
            return Some("");
        }
    }

    None
}

/// Return a list of built-in skill directories that ship with the binary.
///
/// In the TypeScript codebase these are registered programmatically in
/// `src/skills/bundled/index.ts`. The Rust equivalent will register them
/// here. For now this returns an empty list — actual bundled skill
/// registration will be added as individual skills are ported.
pub fn builtin_skill_names() -> Vec<&'static str> {
    vec![
        "commit",
        "simplify",
        "update-config",
        "keybindings-help",
        "claude-api",
    ]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_input() {
        let parsed = parse_skill_file("");
        assert!(parsed.frontmatter.name.is_none());
        assert!(parsed.content.is_empty());
    }

    #[test]
    fn parse_no_frontmatter() {
        let md = "# Hello\n\nSome content here.";
        let parsed = parse_skill_file(md);
        assert!(parsed.frontmatter.name.is_none());
        assert_eq!(parsed.content, md);
    }

    #[test]
    fn parse_with_frontmatter() {
        let md = r#"---
name: my-skill
description: A test skill
argument-hint: "<message>"
when_to_use: When the user asks to test
allowed-tools:
  - Read
  - Write
user-invocable: true
disable-model-invocation: false
---
# Prompt

Do the thing.
"#;
        let parsed = parse_skill_file(md);
        assert_eq!(parsed.frontmatter.name.as_deref(), Some("my-skill"));
        assert_eq!(parsed.frontmatter.description.as_deref(), Some("A test skill"));
        assert_eq!(
            parsed.frontmatter.argument_hint.as_deref(),
            Some("<message>")
        );
        assert_eq!(
            parsed.frontmatter.when_to_use.as_deref(),
            Some("When the user asks to test")
        );
        assert_eq!(parsed.frontmatter.allowed_tools, vec!["Read", "Write"]);
        assert_eq!(parsed.frontmatter.user_invocable, Some(true));
        assert_eq!(parsed.frontmatter.disable_model_invocation, Some(false));
        assert!(parsed.content.contains("# Prompt"));
        assert!(parsed.content.contains("Do the thing."));
    }

    #[test]
    fn parse_quoted_values() {
        let md = "---\nname: \"quoted name\"\ndescription: 'single quoted'\n---\nbody";
        let parsed = parse_skill_file(md);
        assert_eq!(parsed.frontmatter.name.as_deref(), Some("quoted name"));
        assert_eq!(
            parsed.frontmatter.description.as_deref(),
            Some("single quoted")
        );
    }

    #[test]
    fn parse_allowed_tools_csv() {
        let md = "---\nallowed-tools: Read, Write, Bash\n---\ncontent";
        let parsed = parse_skill_file(md);
        assert_eq!(
            parsed.frontmatter.allowed_tools,
            vec!["Read", "Write", "Bash"]
        );
    }

    #[test]
    fn match_skill_explicit() {
        let skill = Skill {
            name: "commit".to_string(),
            description: "Commit changes".to_string(),
            content: String::new(),
            source: SkillSource::Builtin,
            argument_hint: None,
            when_to_use: None,
            allowed_tools: vec![],
            user_invocable: true,
            disable_model_invocation: false,
        };

        assert_eq!(match_skill("/commit", &skill), Some(""));
        assert_eq!(match_skill("/commit fix typo", &skill), Some("fix typo"));
        assert_eq!(match_skill("/other", &skill), None);
        assert_eq!(match_skill("commit", &skill), None); // no slash
    }

    #[test]
    fn match_skill_fuzzy() {
        let skill = Skill {
            name: "commit".to_string(),
            description: "Commit changes".to_string(),
            content: String::new(),
            source: SkillSource::Builtin,
            argument_hint: None,
            when_to_use: Some("When the user wants to commit".to_string()),
            allowed_tools: vec![],
            user_invocable: true,
            disable_model_invocation: false,
        };

        // Fuzzy match: input contains the word "commit"
        assert_eq!(match_skill("please commit my changes", &skill), Some(""));
        // No match: "committed" is not an exact word match for "commit"
        assert_eq!(match_skill("I committed earlier", &skill), None);
    }
}
