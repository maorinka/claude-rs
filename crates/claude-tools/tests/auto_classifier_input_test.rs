use claude_tools::agent_tool::AgentTool;
use claude_tools::bash::BashTool;
use claude_tools::edit::FileEditTool;
use claude_tools::glob_tool::GlobTool;
use claude_tools::grep::GrepTool;
use claude_tools::powershell::PowerShellTool;
use claude_tools::read::FileReadTool;
use claude_tools::registry::ToolExecutor;
use claude_tools::task_tools::{TaskCreateTool, TaskGetTool, TaskOutputTool, TaskUpdateTool};
use claude_tools::web_fetch::WebFetchTool;
use claude_tools::write::FileWriteTool;
use serde_json::json;

#[test]
fn shell_tools_project_command_like_ts() {
    assert_eq!(
        BashTool::new()
            .to_auto_classifier_input(&json!({"command": "git status"}))
            .as_deref(),
        Some("git status")
    );
    assert_eq!(
        PowerShellTool
            .to_auto_classifier_input(&json!({"command": "Get-Process"}))
            .as_deref(),
        Some("Get-Process")
    );
}

#[test]
fn file_tools_project_paths_and_new_content_like_ts() {
    assert_eq!(
        FileReadTool
            .to_auto_classifier_input(&json!({"file_path": "src/lib.rs"}))
            .as_deref(),
        Some("src/lib.rs")
    );
    assert_eq!(
        FileEditTool
            .to_auto_classifier_input(&json!({
                "file_path": "src/lib.rs",
                "new_string": "new"
            }))
            .as_deref(),
        Some("src/lib.rs: new")
    );
    assert_eq!(
        FileWriteTool
            .to_auto_classifier_input(&json!({
                "file_path": "src/lib.rs",
                "content": "body"
            }))
            .as_deref(),
        Some("src/lib.rs: body")
    );
}

#[test]
fn search_and_fetch_tools_project_ts_classifier_text() {
    assert_eq!(
        GlobTool
            .to_auto_classifier_input(&json!({"pattern": "**/*.rs"}))
            .as_deref(),
        Some("**/*.rs")
    );
    assert_eq!(
        GrepTool
            .to_auto_classifier_input(&json!({"pattern": "needle", "path": "src"}))
            .as_deref(),
        Some("needle in src")
    );
    assert_eq!(
        WebFetchTool
            .to_auto_classifier_input(&json!({"url": "https://example.com", "prompt": "summarize"}))
            .as_deref(),
        Some("https://example.com: summarize")
    );
}

#[test]
fn task_tools_project_ts_classifier_text() {
    assert_eq!(
        TaskCreateTool
            .to_auto_classifier_input(&json!({"subject": "Implement x"}))
            .as_deref(),
        Some("Implement x")
    );
    assert_eq!(
        TaskUpdateTool
            .to_auto_classifier_input(&json!({
                "taskId": "3",
                "status": "in_progress",
                "subject": "Implement x"
            }))
            .as_deref(),
        Some("3 in_progress Implement x")
    );
    assert_eq!(
        TaskGetTool
            .to_auto_classifier_input(&json!({"taskId": "3"}))
            .as_deref(),
        Some("3")
    );
    assert_eq!(
        TaskOutputTool
            .to_auto_classifier_input(&json!({"task_id": "3"}))
            .as_deref(),
        Some("3")
    );
}

#[test]
fn agent_tool_projects_prompt_with_ts_tags() {
    assert_eq!(
        AgentTool
            .to_auto_classifier_input(&json!({"prompt": "inspect auth"}))
            .as_deref(),
        Some(": inspect auth")
    );
    assert_eq!(
        AgentTool
            .to_auto_classifier_input(&json!({
                "subagent_type": "Explore",
                "mode": "plan",
                "prompt": "map the project"
            }))
            .as_deref(),
        Some("(Explore, mode=plan): map the project")
    );
}
