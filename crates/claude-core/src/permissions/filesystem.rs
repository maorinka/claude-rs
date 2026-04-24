use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use once_cell::sync::Lazy;
use regex::Regex;

use super::types::{
    PermissionAskDecision, PermissionBehavior, PermissionDecision, PermissionDecisionReason,
    PermissionDenyDecision, PermissionRule, PermissionRuleSource, PermissionRuleValue,
    PermissionUpdate, ToolPermissionContext,
};

// ============================================================================
// Dangerous Files & Directories
// ============================================================================

/// Files that should be protected from auto-editing.
/// These can be used for code execution or data exfiltration.
pub const DANGEROUS_FILES: &[&str] = &[
    ".gitconfig",
    ".gitmodules",
    ".bashrc",
    ".bash_profile",
    ".zshrc",
    ".zprofile",
    ".profile",
    ".ripgreprc",
    ".mcp.json",
    ".claude.json",
];

/// Directories that should be protected from auto-editing.
/// These contain sensitive configuration or executable files.
pub const DANGEROUS_DIRECTORIES: &[&str] = &[".git", ".vscode", ".idea", ".claude"];

// ============================================================================
// Path Normalization
// ============================================================================

/// Normalizes a path for case-insensitive comparison.
/// Prevents bypassing security checks using mixed-case paths on
/// case-insensitive filesystems (macOS/Windows).
pub fn normalize_case_for_comparison(path: &str) -> String {
    path.to_lowercase()
}

/// Expand `~` to home directory and resolve `.` / `..` segments.
pub fn expand_path(path: &str) -> PathBuf {
    let expanded = if path.starts_with('~') {
        if let Some(home) = dirs::home_dir() {
            if path == "~" {
                home
            } else if let Some(stripped) = path.strip_prefix("~/") {
                home.join(stripped)
            } else {
                PathBuf::from(path)
            }
        } else {
            PathBuf::from(path)
        }
    } else {
        PathBuf::from(path)
    };

    // Normalize the path (resolve . and .. lexically)
    normalize_path(&expanded)
}

/// Lexical path normalization (no I/O). Resolves `.` and `..` segments.
fn normalize_path(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}, // skip .
            Component::ParentDir => {
                result.pop();
            },
            other => result.push(other),
        }
    }
    if result.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        result
    }
}

/// Check if a relative path contains path traversal (`..`).
pub fn contains_path_traversal(relative: &str) -> bool {
    relative == ".."
        || relative.starts_with("../")
        || relative.contains("/../")
        || relative.ends_with("/..")
}

/// Cross-platform relative path calculation returning POSIX-style paths.
pub fn relative_path(from: &str, to: &str) -> String {
    let from_path = PathBuf::from(from);
    let to_path = PathBuf::from(to);

    // Use pathdiff for relative path calculation
    if let Some(rel) = pathdiff_relative(&from_path, &to_path) {
        // Convert to forward slashes for POSIX compatibility
        rel.to_string_lossy().replace('\\', "/")
    } else {
        to.to_string()
    }
}

/// Simple relative path calculation (no I/O).
fn pathdiff_relative(base: &Path, target: &Path) -> Option<PathBuf> {
    let base_components: Vec<Component> = base.components().collect();
    let target_components: Vec<Component> = target.components().collect();

    // Find common prefix length
    let common_len = base_components
        .iter()
        .zip(target_components.iter())
        .take_while(|(a, b)| a == b)
        .count();

    let mut result = PathBuf::new();

    // Add .. for each remaining base component
    for _ in common_len..base_components.len() {
        result.push("..");
    }

    // Add remaining target components
    for component in &target_components[common_len..] {
        result.push(component);
    }

    Some(result)
}

/// Get directory for a given path (parent directory).
pub fn get_directory_for_path(path: &str) -> String {
    let p = PathBuf::from(path);
    p.parent()
        .map(|parent| parent.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string())
}

// ============================================================================
// Path Safety Checks
// ============================================================================

/// Check if a file path is dangerous to auto-edit without explicit permission.
/// This includes files in .git, .vscode, .idea, .claude directories and
/// shell configuration files.
fn is_dangerous_file_path_to_auto_edit(path: &str) -> bool {
    let absolute_path = expand_path(path);
    let path_str = absolute_path.to_string_lossy();
    let path_segments: Vec<&str> = path_str.split('/').collect();
    let file_name = path_segments.last().copied();

    // Check for UNC paths (defense-in-depth)
    if path.starts_with("\\\\") || path.starts_with("//") {
        return true;
    }

    // Check if path is within dangerous directories (case-insensitive)
    for (i, segment) in path_segments.iter().enumerate() {
        let normalized_segment = normalize_case_for_comparison(segment);

        for dir in DANGEROUS_DIRECTORIES {
            let normalized_dir = normalize_case_for_comparison(dir);
            if normalized_segment != normalized_dir {
                continue;
            }

            // Special case: .claude/worktrees/ is structural, not dangerous.
            if *dir == ".claude" {
                if let Some(next_segment) = path_segments.get(i + 1) {
                    if normalize_case_for_comparison(next_segment) == "worktrees" {
                        break; // Skip this .claude, continue checking other segments
                    }
                }
            }

            return true;
        }
    }

    // Check for dangerous configuration files (case-insensitive)
    if let Some(fname) = file_name {
        let normalized_name = normalize_case_for_comparison(fname);
        if DANGEROUS_FILES
            .iter()
            .any(|f| normalize_case_for_comparison(f) == normalized_name)
        {
            return true;
        }
    }

    false
}

static SHORT_NAME_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"~\d").unwrap());
static DOS_DEVICE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\.(CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])$").unwrap());
static TRIPLE_DOT_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(^|/|\\)\.{3,}(/|\\|$)").unwrap());

/// Detect suspicious Windows path patterns that could bypass security checks.
/// Includes NTFS ADS, 8.3 short names, long path prefixes, trailing dots/spaces,
/// DOS device names, and triple-dot sequences.
fn has_suspicious_windows_path_pattern(path: &str) -> bool {
    // Check for 8.3 short names (~ followed by digit)
    if path.contains('~') && SHORT_NAME_RE.is_match(path) {
        return true;
    }

    // Check for long path prefixes
    if path.starts_with("\\\\?\\")
        || path.starts_with("\\\\.\\")
        || path.starts_with("//?/")
        || path.starts_with("//./")
    {
        return true;
    }

    // Check for trailing dots and spaces
    if path.ends_with('.') || path.ends_with(' ') {
        return true;
    }
    // More general: trailing dots/spaces
    let trimmed = path.trim_end_matches(|c: char| c == '.' || c.is_whitespace());
    if trimmed.len() != path.len() && !path.is_empty() {
        return true;
    }

    // Check for DOS device names at end of path
    if DOS_DEVICE_RE.is_match(path) {
        return true;
    }

    // Check for three or more consecutive dots as path component
    if TRIPLE_DOT_RE.is_match(path) {
        return true;
    }

    // Check for UNC paths
    if contains_vulnerable_unc_path(path) {
        return true;
    }

    false
}

/// Check for UNC path patterns that could access network resources.
fn contains_vulnerable_unc_path(path: &str) -> bool {
    path.starts_with("\\\\") || path.starts_with("//")
}

/// Check if a path is within a Claude settings path.
pub fn is_claude_settings_path(file_path: &str, cwd: &Path) -> bool {
    let expanded = expand_path(file_path);
    let normalized = normalize_case_for_comparison(&expanded.to_string_lossy());
    let sep = std::path::MAIN_SEPARATOR;

    // Check for .claude/settings.json and .claude/settings.local.json
    let settings_suffix = format!("{}.claude{}settings.json", sep, sep);
    let local_settings_suffix = format!("{}.claude{}settings.local.json", sep, sep);
    if normalized.ends_with(&settings_suffix) || normalized.ends_with(&local_settings_suffix) {
        return true;
    }

    // Check for current project's settings files
    let project_settings = cwd.join(".claude").join("settings.json");
    let project_local_settings = cwd.join(".claude").join("settings.local.json");
    let normalized_project = normalize_case_for_comparison(&project_settings.to_string_lossy());
    let normalized_local = normalize_case_for_comparison(&project_local_settings.to_string_lossy());

    if normalized == normalized_project || normalized == normalized_local {
        return true;
    }

    // Check user-level settings
    if let Some(home) = dirs::home_dir() {
        let user_settings = home.join(".claude").join("settings.json");
        let user_local = home.join(".claude").join("settings.local.json");
        let normalized_user = normalize_case_for_comparison(&user_settings.to_string_lossy());
        let normalized_user_local = normalize_case_for_comparison(&user_local.to_string_lossy());
        if normalized == normalized_user || normalized == normalized_user_local {
            return true;
        }
    }

    false
}

/// Check if a path is within Claude config file paths (settings, commands, agents, skills).
fn is_claude_config_file_path(file_path: &str, cwd: &Path) -> bool {
    if is_claude_settings_path(file_path, cwd) {
        return true;
    }

    let commands_dir = cwd.join(".claude").join("commands");
    let agents_dir = cwd.join(".claude").join("agents");
    let skills_dir = cwd.join(".claude").join("skills");

    path_in_working_path(file_path, &commands_dir.to_string_lossy())
        || path_in_working_path(file_path, &agents_dir.to_string_lossy())
        || path_in_working_path(file_path, &skills_dir.to_string_lossy())
}

// ============================================================================
// checkPathSafetyForAutoEdit
// ============================================================================

/// Safety check result for auto-editing.
pub enum PathSafetyResult {
    Safe,
    Unsafe {
        message: String,
        classifier_approvable: bool,
    },
}

/// Checks if a path is safe for auto-editing (acceptEdits mode).
/// Performs comprehensive safety checks including:
/// - Suspicious Windows path patterns
/// - Claude config files
/// - Dangerous files/directories
///
/// The `cwd` parameter is used for resolving project-relative paths.
pub fn check_path_safety_for_auto_edit(path: &str, cwd: &Path) -> PathSafetyResult {
    let paths_to_check = get_paths_for_permission_check(path);

    // Check for suspicious Windows path patterns
    for p in &paths_to_check {
        if has_suspicious_windows_path_pattern(p) {
            return PathSafetyResult::Unsafe {
                message: format!(
                    "Claude requested permissions to write to {}, which contains a suspicious \
                     Windows path pattern that requires manual approval.",
                    path
                ),
                classifier_approvable: false,
            };
        }
    }

    // Check for Claude config files
    for p in &paths_to_check {
        if is_claude_config_file_path(p, cwd) {
            return PathSafetyResult::Unsafe {
                message: format!(
                    "Claude requested permissions to write to {}, but you haven't granted it yet.",
                    path
                ),
                classifier_approvable: true,
            };
        }
    }

    // Check for dangerous files
    for p in &paths_to_check {
        if is_dangerous_file_path_to_auto_edit(p) {
            return PathSafetyResult::Unsafe {
                message: format!(
                    "Claude requested permissions to edit {} which is a sensitive file.",
                    path
                ),
                classifier_approvable: true,
            };
        }
    }

    PathSafetyResult::Safe
}

// ============================================================================
// Working Path Checks
// ============================================================================

/// Get all working directories from context (cwd + additional).
pub fn all_working_directories(context: &ToolPermissionContext) -> Vec<String> {
    let mut dirs = vec![context.working_directory.to_string_lossy().to_string()];
    for key in context.additional_working_directories.keys() {
        dirs.push(key.clone());
    }
    dirs
}

/// Get paths to check for permission (original path + resolved symlink paths).
/// In the Rust implementation we return the expanded path and, if it differs,
/// the original. Symlink resolution would require I/O; callers can extend this.
pub fn get_paths_for_permission_check(path: &str) -> Vec<String> {
    let expanded = expand_path(path);
    let expanded_str = expanded.to_string_lossy().to_string();

    // Also check the canonical path if it exists (symlink resolution)
    let mut paths = vec![expanded_str.clone()];
    if let Ok(canonical) = std::fs::canonicalize(&expanded) {
        let canonical_str = canonical.to_string_lossy().to_string();
        if canonical_str != expanded_str {
            paths.push(canonical_str);
        }
    }
    paths
}

/// Check if a path is within an allowed working directory.
pub fn path_in_allowed_working_path(path: &str, context: &ToolPermissionContext) -> bool {
    let paths_to_check = get_paths_for_permission_check(path);

    let working_paths: Vec<String> = all_working_directories(context)
        .into_iter()
        .flat_map(|wp| get_paths_for_permission_check(&wp))
        .collect();

    // All resolved paths must be within allowed working paths
    paths_to_check.iter().all(|path_to_check| {
        working_paths
            .iter()
            .any(|wp| path_in_working_path(path_to_check, wp))
    })
}

/// Check if `path` is inside `working_path` (or is the same path).
pub fn path_in_working_path(path: &str, working_path: &str) -> bool {
    let absolute_path = expand_path(path);
    let absolute_working = expand_path(working_path);

    let abs_str = absolute_path.to_string_lossy().to_string();
    let work_str = absolute_working.to_string_lossy().to_string();

    // macOS symlink normalization
    let normalized_path = abs_str
        .replace("/private/var/", "/var/")
        .replace("/private/tmp/", "/tmp/")
        .replace("/private/tmp", "/tmp");
    let normalized_working = work_str
        .replace("/private/var/", "/var/")
        .replace("/private/tmp/", "/tmp/")
        .replace("/private/tmp", "/tmp");

    let case_path = normalize_case_for_comparison(&normalized_path);
    let case_working = normalize_case_for_comparison(&normalized_working);

    let rel = relative_path(&case_working, &case_path);

    // Same path
    if rel.is_empty() {
        return true;
    }

    if contains_path_traversal(&rel) {
        return false;
    }

    // Path is inside (relative path that doesn't go up and isn't absolute)
    !rel.starts_with('/')
}

// ============================================================================
// Rule Matching (gitignore-style)
// ============================================================================

/// Match a file path against a pattern using gitignore-style matching.
/// Supports *, **, ?, and directory matching.
fn gitignore_matches(pattern: &str, path: &str) -> bool {
    // The `ignore` crate in TS uses the npm `ignore` package.
    // We implement basic gitignore glob matching here.
    glob_match(pattern, path)
}

/// Simple glob matching that supports * and ** patterns.
fn glob_match(pattern: &str, text: &str) -> bool {
    glob_match_impl(pattern.as_bytes(), text.as_bytes())
}

fn glob_match_impl(pattern: &[u8], text: &[u8]) -> bool {
    let mut p = 0;
    let mut t = 0;
    let mut star_p = None;
    let mut star_t = 0;

    while t < text.len() {
        if p < pattern.len() && pattern[p] == b'*' {
            if p + 1 < pattern.len() && pattern[p + 1] == b'*' {
                // ** matches everything including /
                // Try matching from current position and all subsequent positions
                let rest = &pattern[p + 2..];
                // Skip optional / after **
                let rest = if !rest.is_empty() && rest[0] == b'/' {
                    &rest[1..]
                } else {
                    rest
                };
                if rest.is_empty() {
                    return true;
                }
                for i in t..=text.len() {
                    if glob_match_impl(rest, &text[i..]) {
                        return true;
                    }
                }
                return false;
            }
            // * matches everything except /
            star_p = Some(p);
            star_t = t;
            p += 1;
        } else if p < pattern.len()
            && (pattern[p] == b'?' || pattern[p] == text[t])
            && text[t] != b'/'
        {
            p += 1;
            t += 1;
        } else if let Some(sp) = star_p {
            // * cannot match /
            if text[star_t] == b'/' {
                return false;
            }
            p = sp + 1;
            star_t += 1;
            t = star_t;
        } else {
            return false;
        }
    }

    // Skip trailing *s
    while p < pattern.len() && pattern[p] == b'*' {
        p += 1;
    }

    p == pattern.len()
}

/// Determine the root for a pattern based on its format and source.
fn pattern_with_root(
    pattern: &str,
    source: &PermissionRuleSource,
    cwd: &Path,
) -> (String, Option<String>) {
    if pattern.starts_with("//") {
        // Patterns starting with // resolve relative to /
        let without_double_slash = &pattern[1..];
        (without_double_slash.to_string(), Some("/".to_string()))
    } else if pattern.starts_with("~/") {
        // Patterns starting with ~/ resolve relative to homedir
        let home = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/"))
            .to_string_lossy()
            .to_string();
        (pattern[1..].to_string(), Some(home))
    } else if pattern.starts_with('/') {
        // Patterns starting with / resolve relative to setting source root
        let root = root_path_for_source(source, cwd);
        (pattern.to_string(), Some(root))
    } else {
        // No root specified
        let mut normalized = pattern.to_string();
        if normalized.starts_with("./") {
            normalized = normalized[2..].to_string();
        }
        (normalized, None)
    }
}

/// Get the root path for a rule source.
fn root_path_for_source(source: &PermissionRuleSource, cwd: &Path) -> String {
    match source {
        PermissionRuleSource::CliArg
        | PermissionRuleSource::Command
        | PermissionRuleSource::Session => cwd.to_string_lossy().to_string(),
        PermissionRuleSource::UserSettings | PermissionRuleSource::PolicySettings => {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("/"))
                .to_string_lossy()
                .to_string()
        },
        PermissionRuleSource::ProjectSettings | PermissionRuleSource::LocalSettings => {
            cwd.to_string_lossy().to_string()
        },
        PermissionRuleSource::FlagSettings => cwd.to_string_lossy().to_string(),
    }
}

/// Get the rules for a specific tool and behavior, keyed by rule content.
pub fn get_rule_by_contents_for_tool_name(
    context: &ToolPermissionContext,
    tool_name: &str,
    behavior: &PermissionBehavior,
) -> HashMap<String, PermissionRule> {
    let mut result = HashMap::new();
    let rules_by_source = context.rules_for_behavior(behavior);

    for source in PermissionRuleSource::all_sources() {
        if let Some(rule_strings) = rules_by_source.get(source) {
            for rule_string in rule_strings {
                let rule_value = PermissionRuleValue::from_string(rule_string);
                if rule_value.tool_name == tool_name {
                    if let Some(ref content) = rule_value.rule_content {
                        let content_key = content.clone();
                        let rule = PermissionRule {
                            source: source.clone(),
                            rule_behavior: behavior.clone(),
                            rule_value,
                        };
                        result.insert(content_key, rule);
                    }
                }
            }
        }
    }

    result
}

/// Find a matching rule for a file path against content-specific rules for a tool.
pub fn matching_rule_for_input(
    path: &str,
    context: &ToolPermissionContext,
    tool_type: ToolType,
    behavior: &PermissionBehavior,
) -> Option<PermissionRule> {
    let tool_name = match tool_type {
        ToolType::Edit => "Edit",
        ToolType::Read => "Read",
    };

    let rules = get_rule_by_contents_for_tool_name(context, tool_name, behavior);
    let file_absolute_path = expand_path(path);
    let file_path_str = file_absolute_path.to_string_lossy().to_string();
    let cwd = &context.working_directory;

    for (pattern_str, rule) in &rules {
        let (relative_pattern, root) = pattern_with_root(pattern_str, &rule.source, cwd);

        // Adjust pattern: remove /** suffix (ignore library treats 'path' as matching both)
        let mut adjusted_pattern = relative_pattern.clone();
        if adjusted_pattern.ends_with("/**") {
            adjusted_pattern = adjusted_pattern[..adjusted_pattern.len() - 3].to_string();
        }

        let cwd_string = cwd.to_string_lossy().to_string();
        let root_str = root.as_deref().unwrap_or(&cwd_string);
        let rel_path = relative_path(root_str, &file_path_str);

        if rel_path.starts_with("../") || rel_path == ".." {
            continue;
        }

        if rel_path.is_empty() {
            continue;
        }

        // Try matching the pattern against the relative path
        // Also prepend / for the gitignore convention
        let test_path = if rel_path.starts_with('/') {
            rel_path.clone()
        } else {
            format!("/{}", rel_path)
        };
        let test_pattern = if adjusted_pattern.starts_with('/') {
            adjusted_pattern.clone()
        } else {
            format!("/{}", adjusted_pattern)
        };

        if gitignore_matches(&test_pattern, &test_path) {
            return Some(rule.clone());
        }

        // Also try matching without the leading /
        if gitignore_matches(&adjusted_pattern, &rel_path) {
            return Some(rule.clone());
        }

        // Check if the original (with /**) also matches
        let original_with_wildcard = format!("{}/", adjusted_pattern);
        if rel_path.starts_with(&original_with_wildcard)
            || gitignore_matches(&format!("{}/**", adjusted_pattern), &rel_path)
        {
            return Some(rule.clone());
        }
    }

    None
}

/// Tool type for permission checking.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolType {
    Edit,
    Read,
}

// ============================================================================
// File Read Ignore Patterns
// ============================================================================

/// Collect all deny rules for file read permissions and return their ignore patterns.
/// Each pattern is grouped by its root directory.
/// This is used to hide files blocked by Read deny rules.
pub fn get_file_read_ignore_patterns(
    context: &ToolPermissionContext,
) -> HashMap<Option<String>, Vec<String>> {
    let rules = get_rule_by_contents_for_tool_name(context, "Read", &PermissionBehavior::Deny);
    let cwd = &context.working_directory;
    let mut result: HashMap<Option<String>, Vec<String>> = HashMap::new();

    for (pattern_str, rule) in &rules {
        let (relative_pattern, root) = pattern_with_root(pattern_str, &rule.source, cwd);
        result.entry(root).or_default().push(relative_pattern);
    }

    result
}

/// Normalize patterns from multiple roots to be relative to a single root.
pub fn normalize_patterns_to_path(
    patterns_by_root: &HashMap<Option<String>, Vec<String>>,
    root: &str,
) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();

    // null root means the pattern can match anywhere
    if let Some(patterns) = patterns_by_root.get(&None) {
        result.extend(patterns.clone());
    }

    for (pattern_root, patterns) in patterns_by_root {
        if pattern_root.is_none() {
            continue;
        }
        let pattern_root = pattern_root.as_ref().unwrap();

        for pattern in patterns {
            let full_pattern = format!(
                "{}/{}",
                pattern_root.trim_end_matches('/'),
                pattern.trim_start_matches('/')
            );
            if pattern_root == root {
                result.push(format!("/{}", pattern.trim_start_matches('/')));
            } else if full_pattern.starts_with(&format!("{}/", root)) {
                let relative_part = &full_pattern[root.len()..];
                result.push(format!("/{}", relative_part.trim_start_matches('/')));
            } else {
                let rel = relative_path(root, pattern_root);
                if rel.is_empty() || rel.starts_with("../") || rel == ".." {
                    continue;
                }
                let rel_pattern = format!("{}/{}", rel, pattern.trim_start_matches('/'));
                result.push(format!("/{}", rel_pattern.trim_start_matches('/')));
            }
        }
    }

    result.sort();
    result.dedup();
    result
}

// ============================================================================
// Read Permission Check
// ============================================================================

/// Permission result for read permission for the specified tool and input path.
pub fn check_read_permission_for_tool(
    path: &str,
    context: &ToolPermissionContext,
) -> PermissionDecision {
    let paths_to_check = get_paths_for_permission_check(path);

    // 1. Block UNC paths early
    for p in &paths_to_check {
        if p.starts_with("\\\\") || p.starts_with("//") {
            return PermissionDecision::Ask(PermissionAskDecision {
                message: format!(
                    "Claude requested permissions to read from {}, which appears to be a UNC path \
                     that could access network resources.",
                    path
                ),
                updated_input: None,
                decision_reason: Some(PermissionDecisionReason::Other {
                    reason: "UNC path detected (defense-in-depth check)".to_string(),
                }),
                suggestions: None,
                blocked_path: None,
                is_bash_security_check_for_misparsing: None,
            });
        }
    }

    // 2. Suspicious Windows patterns
    for p in &paths_to_check {
        if has_suspicious_windows_path_pattern(p) {
            return PermissionDecision::Ask(PermissionAskDecision {
                message: format!(
                    "Claude requested permissions to read from {}, which contains a suspicious \
                     Windows path pattern that requires manual approval.",
                    path
                ),
                updated_input: None,
                decision_reason: Some(PermissionDecisionReason::Other {
                    reason: "Path contains suspicious Windows-specific patterns".to_string(),
                }),
                suggestions: None,
                blocked_path: None,
                is_bash_security_check_for_misparsing: None,
            });
        }
    }

    // 3. Read-specific deny rules
    for p in &paths_to_check {
        if let Some(deny_rule) =
            matching_rule_for_input(p, context, ToolType::Read, &PermissionBehavior::Deny)
        {
            return PermissionDecision::Deny(PermissionDenyDecision {
                message: format!("Permission to read {} has been denied.", path),
                decision_reason: PermissionDecisionReason::Rule { rule: deny_rule },
                tool_use_id: None,
            });
        }
    }

    // 4. Read-specific ask rules
    for p in &paths_to_check {
        if let Some(ask_rule) =
            matching_rule_for_input(p, context, ToolType::Read, &PermissionBehavior::Ask)
        {
            return PermissionDecision::Ask(PermissionAskDecision {
                message: format!(
                    "Claude requested permissions to read from {}, but you haven't granted it yet.",
                    path
                ),
                updated_input: None,
                decision_reason: Some(PermissionDecisionReason::Rule { rule: ask_rule }),
                suggestions: None,
                blocked_path: None,
                is_bash_security_check_for_misparsing: None,
            });
        }
    }

    // 5. Edit access implies read access
    let edit_result = check_write_permission_for_tool(path, context);
    if edit_result.is_allow() {
        return edit_result;
    }

    // 6. Allow reads in working directories
    if path_in_allowed_working_path(path, context) {
        return PermissionDecision::allow_with_reason(PermissionDecisionReason::Mode {
            mode: PermissionMode::Default,
        });
    }

    // 7. Check for read allow rules
    for p in &paths_to_check {
        if let Some(allow_rule) =
            matching_rule_for_input(p, context, ToolType::Read, &PermissionBehavior::Allow)
        {
            return PermissionDecision::allow_with_reason(PermissionDecisionReason::Rule {
                rule: allow_rule,
            });
        }
    }

    // 8. Default: ask for permission
    PermissionDecision::Ask(PermissionAskDecision {
        message: format!(
            "Claude requested permissions to read from {}, but you haven't granted it yet.",
            path
        ),
        updated_input: None,
        decision_reason: Some(PermissionDecisionReason::WorkingDir {
            reason: "Path is outside allowed working directories".to_string(),
        }),
        suggestions: Some(generate_suggestions(path, "read", context)),
        blocked_path: None,
        is_bash_security_check_for_misparsing: None,
    })
}

// We need PermissionMode to be available. Import it via the super path.
use super::types::PermissionMode;

// ============================================================================
// Write Permission Check
// ============================================================================

/// Permission result for write permission for the specified tool and input path.
pub fn check_write_permission_for_tool(
    path: &str,
    context: &ToolPermissionContext,
) -> PermissionDecision {
    let paths_to_check = get_paths_for_permission_check(path);
    let cwd = &context.working_directory;

    // 1. Check deny rules
    for p in &paths_to_check {
        if let Some(deny_rule) =
            matching_rule_for_input(p, context, ToolType::Edit, &PermissionBehavior::Deny)
        {
            return PermissionDecision::Deny(PermissionDenyDecision {
                message: format!("Permission to edit {} has been denied.", path),
                decision_reason: PermissionDecisionReason::Rule { rule: deny_rule },
                tool_use_id: None,
            });
        }
    }

    // 1.6. Check for .claude/** session allow rules before safety checks
    if let Some(allow_rule) = matching_rule_for_session_claude_folder(path, context) {
        let rule_content = allow_rule.rule_value.rule_content.as_deref().unwrap_or("");
        if (rule_content.starts_with("/.claude/") || rule_content.starts_with("~/.claude/"))
            && !rule_content.contains("..")
            && rule_content.ends_with("/**")
        {
            return PermissionDecision::allow_with_reason(PermissionDecisionReason::Rule {
                rule: allow_rule,
            });
        }
    }

    // 1.7. Safety checks (Windows patterns, Claude config, dangerous files)
    let safety = check_path_safety_for_auto_edit(path, cwd);
    if let PathSafetyResult::Unsafe {
        message,
        classifier_approvable,
    } = safety
    {
        return PermissionDecision::Ask(PermissionAskDecision {
            message: message.clone(),
            updated_input: None,
            decision_reason: Some(PermissionDecisionReason::SafetyCheck {
                reason: message,
                classifier_approvable,
            }),
            suggestions: Some(generate_suggestions(path, "write", context)),
            blocked_path: None,
            is_bash_security_check_for_misparsing: None,
        });
    }

    // 2. Check ask rules
    for p in &paths_to_check {
        if let Some(ask_rule) =
            matching_rule_for_input(p, context, ToolType::Edit, &PermissionBehavior::Ask)
        {
            return PermissionDecision::Ask(PermissionAskDecision {
                message: format!(
                    "Claude requested permissions to write to {}, but you haven't granted it yet.",
                    path
                ),
                updated_input: None,
                decision_reason: Some(PermissionDecisionReason::Rule { rule: ask_rule }),
                suggestions: None,
                blocked_path: None,
                is_bash_security_check_for_misparsing: None,
            });
        }
    }

    // 3. In acceptEdits mode, allow all writes in working directory
    let is_in_working_dir = path_in_allowed_working_path(path, context);
    if context.mode == PermissionMode::AcceptEdits && is_in_working_dir {
        return PermissionDecision::allow_with_reason(PermissionDecisionReason::Mode {
            mode: context.mode.clone(),
        });
    }

    // 4. Check allow rules
    if let Some(allow_rule) =
        matching_rule_for_input(path, context, ToolType::Edit, &PermissionBehavior::Allow)
    {
        return PermissionDecision::allow_with_reason(PermissionDecisionReason::Rule {
            rule: allow_rule,
        });
    }

    // 5. Default: ask
    let reason = if !is_in_working_dir {
        Some(PermissionDecisionReason::WorkingDir {
            reason: "Path is outside allowed working directories".to_string(),
        })
    } else {
        None
    };

    PermissionDecision::Ask(PermissionAskDecision {
        message: format!(
            "Claude requested permissions to write to {}, but you haven't granted it yet.",
            path
        ),
        updated_input: None,
        decision_reason: reason,
        suggestions: Some(generate_suggestions(path, "write", context)),
        blocked_path: None,
        is_bash_security_check_for_misparsing: None,
    })
}

/// Check for .claude/** session-only allow rules.
fn matching_rule_for_session_claude_folder(
    path: &str,
    context: &ToolPermissionContext,
) -> Option<PermissionRule> {
    // Create a temporary context with only session allow rules
    let mut session_only = ToolPermissionContext {
        mode: context.mode.clone(),
        additional_working_directories: context.additional_working_directories.clone(),
        always_allow_rules: HashMap::new(),
        always_deny_rules: context.always_deny_rules.clone(),
        always_ask_rules: context.always_ask_rules.clone(),
        is_bypass_permissions_mode_available: context.is_bypass_permissions_mode_available,
        stripped_dangerous_rules: context.stripped_dangerous_rules.clone(),
        should_avoid_permission_prompts: context.should_avoid_permission_prompts,
        is_auto_mode_available: context.is_auto_mode_available,
        pre_plan_mode: context.pre_plan_mode.clone(),
        working_directory: context.working_directory.clone(),
    };

    if let Some(session_rules) = context
        .always_allow_rules
        .get(&PermissionRuleSource::Session)
    {
        session_only
            .always_allow_rules
            .insert(PermissionRuleSource::Session, session_rules.clone());
    }

    matching_rule_for_input(
        path,
        &session_only,
        ToolType::Edit,
        &PermissionBehavior::Allow,
    )
}

// ============================================================================
// Suggestion Generation
// ============================================================================

/// Generate permission update suggestions for a blocked path.
pub fn generate_suggestions(
    file_path: &str,
    operation_type: &str,
    context: &ToolPermissionContext,
) -> Vec<PermissionUpdate> {
    let is_outside_working_dir = !path_in_allowed_working_path(file_path, context);

    // Only suggest acceptEdits when it would be an upgrade
    let should_suggest_accept_edits =
        matches!(context.mode, PermissionMode::Default | PermissionMode::Plan);

    if operation_type == "read" && is_outside_working_dir {
        let dir_path = get_directory_for_path(file_path);
        let dirs_to_add = get_paths_for_permission_check(&dir_path);
        let suggestions: Vec<PermissionUpdate> = dirs_to_add
            .into_iter()
            .map(|dir| PermissionUpdate::AddRules {
                destination: PermissionRuleSource::Session,
                rules: vec![PermissionRuleValue {
                    tool_name: "Read".to_string(),
                    rule_content: Some(format!("{}/**", dir)),
                }],
                behavior: PermissionBehavior::Allow,
            })
            .collect();
        return suggestions;
    }

    if operation_type == "write" || operation_type == "create" {
        let mut updates: Vec<PermissionUpdate> = if should_suggest_accept_edits {
            vec![PermissionUpdate::SetMode {
                mode: PermissionMode::AcceptEdits,
                destination: PermissionRuleSource::Session,
            }]
        } else {
            vec![]
        };

        if is_outside_working_dir {
            let dir_path = get_directory_for_path(file_path);
            let dirs_to_add = get_paths_for_permission_check(&dir_path);
            updates.push(PermissionUpdate::AddDirectories {
                directories: dirs_to_add,
                destination: PermissionRuleSource::Session,
            });
        }

        return updates;
    }

    // For read operations inside working directories
    if should_suggest_accept_edits {
        vec![PermissionUpdate::SetMode {
            mode: PermissionMode::AcceptEdits,
            destination: PermissionRuleSource::Session,
        }]
    } else {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dangerous_files_list() {
        assert!(DANGEROUS_FILES.contains(&".gitconfig"));
        assert!(DANGEROUS_FILES.contains(&".bashrc"));
        assert!(DANGEROUS_FILES.contains(&".mcp.json"));
        assert!(!DANGEROUS_FILES.contains(&"README.md"));
    }

    #[test]
    fn test_dangerous_directories_list() {
        assert!(DANGEROUS_DIRECTORIES.contains(&".git"));
        assert!(DANGEROUS_DIRECTORIES.contains(&".claude"));
        assert!(!DANGEROUS_DIRECTORIES.contains(&"src"));
    }

    #[test]
    fn test_is_dangerous_file_path() {
        assert!(is_dangerous_file_path_to_auto_edit("/home/user/.bashrc"));
        assert!(is_dangerous_file_path_to_auto_edit("/project/.git/config"));
        assert!(is_dangerous_file_path_to_auto_edit(
            "/project/.vscode/settings.json"
        ));
        assert!(!is_dangerous_file_path_to_auto_edit("/project/src/main.rs"));
    }

    #[test]
    fn test_dangerous_file_path_case_insensitive() {
        assert!(is_dangerous_file_path_to_auto_edit("/project/.GIT/config"));
        assert!(is_dangerous_file_path_to_auto_edit("/project/.Git/config"));
        assert!(is_dangerous_file_path_to_auto_edit("/home/user/.BASHRC"));
    }

    #[test]
    fn test_claude_worktrees_exception() {
        // .claude/worktrees/ should NOT be treated as dangerous
        assert!(!is_dangerous_file_path_to_auto_edit(
            "/project/.claude/worktrees/feature/src/main.rs"
        ));
        // But .claude/settings.json should still be dangerous
        assert!(is_dangerous_file_path_to_auto_edit(
            "/project/.claude/settings.json"
        ));
    }

    #[test]
    fn test_path_in_working_path() {
        assert!(path_in_working_path("/project/src/main.rs", "/project"));
        assert!(path_in_working_path("/project", "/project"));
        assert!(!path_in_working_path("/other/file.rs", "/project"));
        assert!(!path_in_working_path("/project/../etc/passwd", "/project"));
    }

    #[test]
    fn test_normalize_case_for_comparison() {
        assert_eq!(
            normalize_case_for_comparison("/Project/Src"),
            "/project/src"
        );
    }

    #[test]
    fn test_expand_path_tilde() {
        if let Some(home) = dirs::home_dir() {
            let expanded = expand_path("~/test/file.rs");
            assert_eq!(expanded, home.join("test/file.rs"));
        }
    }

    #[test]
    fn test_suspicious_windows_patterns() {
        assert!(has_suspicious_windows_path_pattern("file~1.txt"));
        assert!(has_suspicious_windows_path_pattern("\\\\?\\C:\\path"));
        assert!(has_suspicious_windows_path_pattern("file.txt."));
        assert!(has_suspicious_windows_path_pattern("file.txt.CON"));
        assert!(has_suspicious_windows_path_pattern("path/.../file"));
        assert!(!has_suspicious_windows_path_pattern(
            "/normal/path/file.txt"
        ));
    }

    #[test]
    fn test_glob_match() {
        assert!(glob_match("*.rs", "main.rs"));
        assert!(glob_match("src/**", "src/main.rs"));
        assert!(glob_match("src/**/test.rs", "src/a/b/test.rs"));
        assert!(!glob_match("*.rs", "main.ts"));
        assert!(glob_match("src/*.rs", "src/main.rs"));
        assert!(!glob_match("src/*.rs", "src/sub/main.rs"));
    }

    #[test]
    fn test_contains_path_traversal() {
        assert!(contains_path_traversal(".."));
        assert!(contains_path_traversal("../etc/passwd"));
        assert!(contains_path_traversal("foo/../../bar"));
        assert!(!contains_path_traversal("foo/bar"));
        assert!(!contains_path_traversal("foo/bar/baz"));
    }
}
