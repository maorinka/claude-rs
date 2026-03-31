use std::path::Path;
use anyhow::Result;
use serde_json::{json, Value};

use super::git::get_git_context;
use super::environment::build_environment_context;

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
