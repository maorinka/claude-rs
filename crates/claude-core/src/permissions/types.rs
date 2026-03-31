use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum PermissionMode {
    #[default]
    Default,
    Bypass,
    InteractiveOnly,
}

#[derive(Clone, Debug)]
pub enum PermissionDecision {
    Allow,
    Deny { message: String },
    Ask { message: String },
}

#[derive(Clone, Debug)]
pub struct PermissionRule {
    pub tool: String,
    pub pattern: Option<String>,
    pub mode: Option<PermissionRuleMode>,
}

#[derive(Clone, Debug)]
pub enum PermissionRuleMode {
    Read,
    Write,
    Full,
}

#[derive(Clone, Debug, Default)]
pub struct ToolPermissionContext {
    pub mode: PermissionMode,
    pub working_directories: HashMap<String, PathBuf>,
    pub allow_rules: HashMap<String, Vec<PermissionRule>>,
    pub deny_rules: HashMap<String, Vec<PermissionRule>>,
    pub ask_rules: HashMap<String, Vec<PermissionRule>>,
}
