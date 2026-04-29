//! Permission/settings disk loading helpers.
//!
//! This is the Rust-side equivalent of the TS settings pieces used by
//! `applySettingsChange`: read current settings from disk, preserve each
//! permission rule's source, and expose a cheap fingerprint so interactive
//! callers can detect changes without hardcoding individual files.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;

use super::types::{
    PermissionBehavior, PermissionMode, PermissionRule, PermissionRuleSource, PermissionRuleValue,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsFileStamp {
    pub modified: Option<SystemTime>,
    pub len: Option<u64>,
}

pub type SettingsFingerprint = BTreeMap<PathBuf, SettingsFileStamp>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingSource {
    User,
    Project,
    Local,
}

impl SettingSource {
    pub fn defaults() -> [Self; 3] {
        [Self::User, Self::Project, Self::Local]
    }
}

fn merge_json_objects(base: &mut Value, overlay: Value) {
    let (Some(base_obj), Some(overlay_obj)) = (base.as_object_mut(), overlay.as_object()) else {
        *base = overlay;
        return;
    };
    for (key, value) in overlay_obj {
        match (base_obj.get_mut(key), value) {
            (Some(existing), Value::Object(_)) if existing.is_object() => {
                merge_json_objects(existing, value.clone());
            }
            _ => {
                base_obj.insert(key.clone(), value.clone());
            }
        }
    }
}

pub fn raw_settings_paths(project_root: &Path) -> Vec<PathBuf> {
    raw_settings_paths_for_sources(project_root, &SettingSource::defaults())
}

pub fn raw_settings_paths_for_sources(
    project_root: &Path,
    sources: &[SettingSource],
) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for source in sources {
        match source {
            SettingSource::User => {
                if let Ok(path) = crate::config::paths::user_settings_path() {
                    paths.push(path);
                }
            }
            SettingSource::Project => {
                paths.push(project_root.join(".claude").join("settings.json"));
            }
            SettingSource::Local => {
                paths.push(project_root.join(".claude").join("settings.local.json"));
            }
        }
    }
    paths
}

pub fn permission_rule_paths(project_root: &Path) -> Vec<(PermissionRuleSource, PathBuf)> {
    permission_rule_paths_for_sources(project_root, &SettingSource::defaults())
}

pub fn permission_rule_paths_for_sources(
    project_root: &Path,
    sources: &[SettingSource],
) -> Vec<(PermissionRuleSource, PathBuf)> {
    let mut paths = Vec::new();
    for source in sources {
        match source {
            SettingSource::User => {
                if let Ok(user_path) = crate::config::paths::user_settings_path() {
                    paths.push((PermissionRuleSource::UserSettings, user_path));
                }
            }
            SettingSource::Project => paths.push((
                PermissionRuleSource::ProjectSettings,
                project_root.join(".claude").join("settings.json"),
            )),
            SettingSource::Local => paths.push((
                PermissionRuleSource::LocalSettings,
                project_root.join(".claude").join("settings.local.json"),
            )),
        }
    }
    paths
}

pub fn settings_change_fingerprint(project_root: &Path) -> SettingsFingerprint {
    let mut paths = raw_settings_paths(project_root);
    paths.push(crate::remote_managed_settings::get_settings_path());
    for root in enabled_plugin_roots(project_root) {
        paths.push(root.join("hooks").join("hooks.json"));
    }
    paths.sort();
    paths.dedup();

    let mut fingerprint = BTreeMap::new();
    for path in paths {
        let stamp = match std::fs::metadata(&path) {
            Ok(meta) => SettingsFileStamp {
                modified: meta.modified().ok(),
                len: Some(meta.len()),
            },
            Err(_) => SettingsFileStamp {
                modified: None,
                len: None,
            },
        };
        fingerprint.insert(path, stamp);
    }
    fingerprint
}

pub fn load_raw_settings_value(project_root: &Path) -> Value {
    load_raw_settings_value_for_sources(project_root, &SettingSource::defaults())
}

pub fn load_raw_settings_value_for_sources(
    project_root: &Path,
    sources: &[SettingSource],
) -> Value {
    let mut merged = serde_json::json!({});
    for path in raw_settings_paths_for_sources(project_root, sources) {
        let Some(value) = load_settings_json_value(&path) else {
            continue;
        };
        merge_json_objects(&mut merged, value);
    }
    merged
}

pub fn load_raw_settings_value_with_plugin_hooks(project_root: &Path) -> Value {
    load_raw_settings_value_with_plugin_hooks_for_sources(project_root, &SettingSource::defaults())
}

pub fn load_raw_settings_value_with_plugin_hooks_for_sources(
    project_root: &Path,
    sources: &[SettingSource],
) -> Value {
    let mut settings = load_raw_settings_value_for_sources(project_root, sources);
    merge_enabled_plugin_hooks(&mut settings, project_root);
    settings
}

fn enabled_plugin_roots(project_root: &Path) -> Vec<PathBuf> {
    let Ok(claude_dir) = crate::config::paths::claude_dir() else {
        return Vec::new();
    };

    let mut roots = Vec::new();
    for plugin_id in crate::plugins::skill::enabled_plugins_for_project(project_root) {
        let Some((name, source)) = plugin_id.split_once('@') else {
            continue;
        };
        let cache_root = claude_dir
            .join("plugins")
            .join("cache")
            .join(source)
            .join(name);
        let Ok(entries) = std::fs::read_dir(cache_root) else {
            continue;
        };
        let mut versions: Vec<_> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect();
        versions.sort();
        if let Some(root) = versions.pop() {
            roots.push(root);
        }
    }
    roots
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn replace_plugin_root(value: &mut Value, root: &Path) {
    match value {
        Value::String(text) => {
            let had_plugin_root = text.contains("${CLAUDE_PLUGIN_ROOT}");
            let root_text = root.display().to_string();
            *text = text.replace("${CLAUDE_PLUGIN_ROOT}", &root.display().to_string());
            if cfg!(unix) && text.contains(".cmd") && !text.trim_start().starts_with("bash ") {
                *text = format!("bash {}", text);
            }
            if had_plugin_root && !text.contains("CLAUDE_PLUGIN_ROOT=") {
                *text = format!(
                    "CLAUDE_PLUGIN_ROOT={} {}",
                    shell_single_quote(&root_text),
                    text
                );
            }
        }
        Value::Array(items) => {
            for item in items {
                replace_plugin_root(item, root);
            }
        }
        Value::Object(map) => {
            for item in map.values_mut() {
                replace_plugin_root(item, root);
            }
        }
        _ => {}
    }
}

pub fn merge_enabled_plugin_hooks(settings: &mut Value, project_root: &Path) {
    for root in enabled_plugin_roots(project_root) {
        let hooks_path = root.join("hooks").join("hooks.json");
        let Ok(text) = std::fs::read_to_string(hooks_path) else {
            continue;
        };
        let Ok(mut value) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        replace_plugin_root(&mut value, &root);
        merge_json_objects(settings, value);
    }
}

pub fn load_settings_json_value(path: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    let value = serde_json::from_str::<Value>(&text).ok()?;
    value.is_object().then_some(value)
}

pub fn load_permission_settings_value(project_root: &Path) -> Value {
    load_permission_settings_value_for_sources(project_root, &SettingSource::defaults())
}

pub fn load_permission_settings_value_for_sources(
    project_root: &Path,
    sources: &[SettingSource],
) -> Value {
    let user_project_local = load_raw_settings_value_for_sources(project_root, sources);
    if let Some(policy) = crate::remote_managed_settings::load_from_disk() {
        crate::remote_managed_settings::apply_policy_overlay(&user_project_local, &policy)
    } else {
        user_project_local
    }
}

pub fn allow_managed_permission_rules_only() -> bool {
    crate::remote_managed_settings::load_from_disk()
        .and_then(|value| {
            value
                .get("permissions")
                .and_then(|permissions| permissions.get("allowManagedPermissionRulesOnly"))
                .and_then(Value::as_bool)
        })
        .unwrap_or(false)
}

pub fn parse_permission_rules_from_settings_value(
    value: &Value,
    source: PermissionRuleSource,
) -> Vec<PermissionRule> {
    let Some(permissions) = value.get("permissions").and_then(Value::as_object) else {
        return Vec::new();
    };

    let mut rules = Vec::new();
    for (key, behavior) in [
        ("allow", PermissionBehavior::Allow),
        ("deny", PermissionBehavior::Deny),
        ("ask", PermissionBehavior::Ask),
    ] {
        let Some(entries) = permissions.get(key).and_then(Value::as_array) else {
            continue;
        };
        for entry in entries {
            let Some(rule_string) = entry.as_str() else {
                continue;
            };
            rules.push(PermissionRule {
                source: source.clone(),
                rule_behavior: behavior.clone(),
                rule_value: PermissionRuleValue::from_string(rule_string),
            });
        }
    }
    rules
}

pub fn load_permission_rules_from_disk_by_source(project_root: &Path) -> Vec<PermissionRule> {
    let policy = crate::remote_managed_settings::load_from_disk();
    if allow_managed_permission_rules_only() {
        return policy
            .as_ref()
            .map(|value| {
                parse_permission_rules_from_settings_value(
                    value,
                    PermissionRuleSource::PolicySettings,
                )
            })
            .unwrap_or_default();
    }

    let mut rules = Vec::new();
    for (source, path) in permission_rule_paths(project_root) {
        let Some(value) = load_settings_json_value(&path) else {
            continue;
        };
        rules.extend(parse_permission_rules_from_settings_value(&value, source));
    }
    if let Some(policy) = policy {
        rules.extend(parse_permission_rules_from_settings_value(
            &policy,
            PermissionRuleSource::PolicySettings,
        ));
    }
    rules
}

pub fn permission_mode_from_settings_value(value: &Value) -> Option<PermissionMode> {
    value
        .get("permissions")
        .and_then(|permissions| permissions.get("defaultMode"))
        .and_then(Value::as_str)
        .map(PermissionMode::from_string)
}

pub fn permission_additional_directories_from_settings_value(value: &Value) -> Vec<String> {
    value
        .get("permissions")
        .and_then(|permissions| permissions.get("additionalDirectories"))
        .and_then(Value::as_array)
        .map(|dirs| {
            dirs.iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_permission_rules_and_directories() {
        let value = serde_json::json!({
            "permissions": {
                "allow": ["Bash(git status)"],
                "deny": ["Write"],
                "ask": ["Edit"],
                "additionalDirectories": ["/tmp/work", 42, "/tmp/other"]
            }
        });

        let rules =
            parse_permission_rules_from_settings_value(&value, PermissionRuleSource::LocalSettings);
        assert_eq!(rules.len(), 3);
        assert!(rules.iter().any(|rule| {
            rule.source == PermissionRuleSource::LocalSettings
                && rule.rule_behavior == PermissionBehavior::Allow
                && rule.rule_value.to_rule_string() == "Bash(git status)"
        }));
        assert!(rules.iter().any(|rule| {
            rule.rule_behavior == PermissionBehavior::Deny
                && rule.rule_value.to_rule_string() == "Write"
        }));
        assert!(rules.iter().any(|rule| {
            rule.rule_behavior == PermissionBehavior::Ask
                && rule.rule_value.to_rule_string() == "Edit"
        }));

        assert_eq!(
            permission_additional_directories_from_settings_value(&value),
            vec!["/tmp/work".to_string(), "/tmp/other".to_string()]
        );
    }

    #[test]
    fn raw_settings_merge_is_deep_for_objects() {
        let mut base = serde_json::json!({"permissions": {"allow": ["Read"], "deny": ["Write"]}});
        merge_json_objects(
            &mut base,
            serde_json::json!({"permissions": {"allow": ["Bash"]}, "model": "x"}),
        );
        assert_eq!(base["permissions"]["allow"], serde_json::json!(["Bash"]));
        assert_eq!(base["permissions"]["deny"], serde_json::json!(["Write"]));
        assert_eq!(base["model"], serde_json::json!("x"));
    }

    #[test]
    fn setting_sources_filter_project_and_local_paths() {
        let root = Path::new("/tmp/project");
        let paths = raw_settings_paths_for_sources(root, &[SettingSource::Project]);
        assert_eq!(paths, vec![root.join(".claude").join("settings.json")]);

        let paths = permission_rule_paths_for_sources(root, &[SettingSource::Local]);
        assert_eq!(
            paths,
            vec![(
                PermissionRuleSource::LocalSettings,
                root.join(".claude").join("settings.local.json")
            )]
        );
    }
}
