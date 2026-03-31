use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Map, Value};
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

/// Returns the default settings file path: `~/.claude/settings.json`.
pub fn default_settings_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".claude").join("settings.json")
}

fn read_settings_at(path: &PathBuf) -> Result<Map<String, Value>> {
    if !path.exists() {
        return Ok(Map::new());
    }
    let content = std::fs::read_to_string(path)?;
    let v: Value = serde_json::from_str(&content)?;
    match v {
        Value::Object(m) => Ok(m),
        _ => Ok(Map::new()),
    }
}

fn write_settings_at(path: &PathBuf, map: &Map<String, Value>) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_string_pretty(&Value::Object(map.clone()))?;
    std::fs::write(path, content)?;
    Ok(())
}

/// A tool for reading and writing Claude configuration settings.
///
/// `settings_path_override` is `None` in production (uses `~/.claude/settings.json`)
/// and `Some(path)` in tests to point at a temp file.
pub struct ConfigTool {
    pub settings_path_override: Option<PathBuf>,
}

impl ConfigTool {
    /// Creates a new `ConfigTool` that uses the default settings path.
    pub fn new() -> Self {
        Self {
            settings_path_override: None,
        }
    }

    fn settings_path(&self) -> PathBuf {
        self.settings_path_override
            .clone()
            .unwrap_or_else(default_settings_path)
    }
}

impl Default for ConfigTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ToolExecutor for ConfigTool {
    fn name(&self) -> &str {
        "Config"
    }

    fn description(&self) -> String {
        "Get, set, or list Claude configuration settings.".to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["get", "set", "list"],
                    "description": "The action to perform: get a key, set a key, or list all settings."
                },
                "key": {
                    "type": "string",
                    "description": "The settings key to get or set."
                },
                "value": {
                    "type": "string",
                    "description": "The value to set (required for 'set' action)."
                }
            },
            "required": ["action"]
        })
    }

    fn is_read_only(&self, input: &Value) -> bool {
        matches!(input["action"].as_str(), Some("get") | Some("list"))
    }

    fn is_destructive(&self, input: &Value) -> bool {
        input["action"].as_str() == Some("set")
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }

    async fn call(
        &self,
        input: &Value,
        _ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let action = match input["action"].as_str() {
            Some(a) => a,
            None => {
                return Ok(ToolResultData {
                    data: json!({ "error": "missing required field: action" }),
                    is_error: true,
                });
            }
        };

        let path = self.settings_path();

        match action {
            "list" => {
                let settings = match read_settings_at(&path) {
                    Ok(s) => s,
                    Err(e) => {
                        return Ok(ToolResultData {
                            data: json!({ "error": format!("failed to read settings: {e}") }),
                            is_error: true,
                        });
                    }
                };
                Ok(ToolResultData {
                    data: json!({
                        "action": "list",
                        "settings": Value::Object(settings)
                    }),
                    is_error: false,
                })
            }
            "get" => {
                let key = match input["key"].as_str() {
                    Some(k) => k,
                    None => {
                        return Ok(ToolResultData {
                            data: json!({ "error": "missing required field: key" }),
                            is_error: true,
                        });
                    }
                };
                let settings = match read_settings_at(&path) {
                    Ok(s) => s,
                    Err(e) => {
                        return Ok(ToolResultData {
                            data: json!({ "error": format!("failed to read settings: {e}") }),
                            is_error: true,
                        });
                    }
                };
                let value = settings.get(key).cloned().unwrap_or(Value::Null);
                Ok(ToolResultData {
                    data: json!({
                        "action": "get",
                        "key": key,
                        "value": value
                    }),
                    is_error: false,
                })
            }
            "set" => {
                let key = match input["key"].as_str() {
                    Some(k) => k,
                    None => {
                        return Ok(ToolResultData {
                            data: json!({ "error": "missing required field: key" }),
                            is_error: true,
                        });
                    }
                };
                let value = match input.get("value") {
                    Some(v) => v.clone(),
                    None => {
                        return Ok(ToolResultData {
                            data: json!({ "error": "missing required field: value" }),
                            is_error: true,
                        });
                    }
                };
                let mut settings = match read_settings_at(&path) {
                    Ok(s) => s,
                    Err(e) => {
                        return Ok(ToolResultData {
                            data: json!({ "error": format!("failed to read settings: {e}") }),
                            is_error: true,
                        });
                    }
                };
                settings.insert(key.to_string(), value.clone());
                if let Err(e) = write_settings_at(&path, &settings) {
                    return Ok(ToolResultData {
                        data: json!({ "error": format!("failed to write settings: {e}") }),
                        is_error: true,
                    });
                }
                Ok(ToolResultData {
                    data: json!({
                        "action": "set",
                        "key": key,
                        "value": value
                    }),
                    is_error: false,
                })
            }
            other => Ok(ToolResultData {
                data: json!({ "error": format!("unknown action: {other}") }),
                is_error: true,
            }),
        }
    }
}
