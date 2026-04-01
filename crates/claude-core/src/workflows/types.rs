//! Workflow configuration types.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkflowStep {
    Prompt { content: String, #[serde(default)] description: Option<String> },
    Command { command: String, #[serde(default)] cwd: Option<String>, #[serde(default)] description: Option<String> },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct WorkflowFrontmatter {
    #[serde(default)] pub name: Option<String>,
    #[serde(default)] pub description: Option<String>,
    #[serde(default)] pub allowed_tools: Option<Vec<String>>,
    #[serde(flatten)] pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowConfig {
    pub slug: String,
    pub source_path: PathBuf,
    pub source: WorkflowSource,
    pub frontmatter: WorkflowFrontmatter,
    pub steps: Vec<WorkflowStep>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowSource { User, Project, Managed }

#[derive(Debug, Clone)]
pub struct StepResult { pub index: usize, pub success: bool, pub output: String }

#[derive(Debug, Clone)]
pub struct WorkflowResult { pub slug: String, pub step_results: Vec<StepResult>, pub success: bool }
