use claude_tools::task_tools::{
    TaskCreateTool, TaskGetTool, TaskListTool, TaskOutputTool, TaskStopTool, TaskUpdateTool,
};
use claude_tools::registry::{ToolExecutor, ToolUseContext};
use serde_json::json;
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;

fn make_ctx() -> ToolUseContext {
    ToolUseContext {
        working_directory: PathBuf::from("/tmp"),
        read_file_state: std::sync::Arc::new(std::sync::Mutex::new(
            claude_tools::registry::ReadFileState::new(),
        )),
    }
}

async fn create_task(subject: &str, description: &str) -> serde_json::Value {
    let tool = TaskCreateTool;
    let input = json!({ "subject": subject, "description": description });
    let result = tool
        .call(&input, &make_ctx(), CancellationToken::new(), None)
        .await
        .unwrap();
    assert!(!result.is_error, "create should succeed: {:?}", result.data);
    result.data
}

#[tokio::test]
async fn test_task_create_returns_id() {
    let data = create_task("Test subject", "Test description").await;
    let id = data["id"].as_str().expect("id should be a string");
    assert!(!id.is_empty(), "id should not be empty");
    assert_eq!(data["status"], "pending");
    assert_eq!(data["subject"], "Test subject");
    assert_eq!(data["description"], "Test description");
}

#[tokio::test]
async fn test_task_get_returns_created_task() {
    let created = create_task("Get test subject", "Get test desc").await;
    let id = created["id"].as_str().unwrap().to_string();

    let tool = TaskGetTool;
    let input = json!({ "taskId": id });
    let result = tool
        .call(&input, &make_ctx(), CancellationToken::new(), None)
        .await
        .unwrap();

    assert!(!result.is_error, "get should succeed");
    assert_eq!(result.data["id"], id);
    assert_eq!(result.data["subject"], "Get test subject");
}

#[tokio::test]
async fn test_task_get_nonexistent_returns_error() {
    let tool = TaskGetTool;
    let input = json!({ "taskId": "nonexistent-id-12345" });
    let result = tool
        .call(&input, &make_ctx(), CancellationToken::new(), None)
        .await
        .unwrap();

    assert!(result.is_error, "get of nonexistent task should be an error");
}

#[tokio::test]
async fn test_task_list_includes_created_tasks() {
    let created = create_task("List test subject", "List test desc").await;
    let id = created["id"].as_str().unwrap().to_string();

    let tool = TaskListTool;
    let result = tool
        .call(&json!({}), &make_ctx(), CancellationToken::new(), None)
        .await
        .unwrap();

    assert!(!result.is_error);
    let tasks = result.data["tasks"].as_array().expect("tasks should be array");
    let found = tasks.iter().any(|t| t["id"].as_str() == Some(&id));
    assert!(found, "created task should appear in list");
}

#[tokio::test]
async fn test_task_update_status() {
    let created = create_task("Update test subject", "Update test desc").await;
    let id = created["id"].as_str().unwrap().to_string();

    let tool = TaskUpdateTool;
    let input = json!({ "taskId": id, "status": "in_progress" });
    let result = tool
        .call(&input, &make_ctx(), CancellationToken::new(), None)
        .await
        .unwrap();

    assert!(!result.is_error, "update should succeed");
    assert_eq!(result.data["status"], "in_progress");
}

#[tokio::test]
async fn test_task_update_subject_and_description() {
    let created = create_task("Old subject", "Old desc").await;
    let id = created["id"].as_str().unwrap().to_string();

    let tool = TaskUpdateTool;
    let input = json!({
        "taskId": id,
        "subject": "New subject",
        "description": "New desc"
    });
    let result = tool
        .call(&input, &make_ctx(), CancellationToken::new(), None)
        .await
        .unwrap();

    assert!(!result.is_error);
    assert_eq!(result.data["subject"], "New subject");
    assert_eq!(result.data["description"], "New desc");
}

#[tokio::test]
async fn test_task_stop_sets_status_stopped() {
    let created = create_task("Stop test subject", "Stop test desc").await;
    let id = created["id"].as_str().unwrap().to_string();

    let tool = TaskStopTool;
    let input = json!({ "taskId": id });
    let result = tool
        .call(&input, &make_ctx(), CancellationToken::new(), None)
        .await
        .unwrap();

    assert!(!result.is_error, "stop should succeed");
    assert_eq!(result.data["status"], "stopped");

    // Verify via get
    let get_tool = TaskGetTool;
    let get_result = get_tool
        .call(&json!({ "taskId": id }), &make_ctx(), CancellationToken::new(), None)
        .await
        .unwrap();
    assert_eq!(get_result.data["status"], "stopped");
}

#[tokio::test]
async fn test_task_output_returns_description() {
    let created = create_task("Output test subject", "This is the output description").await;
    let id = created["id"].as_str().unwrap().to_string();

    let tool = TaskOutputTool;
    let input = json!({ "taskId": id });
    let result = tool
        .call(&input, &make_ctx(), CancellationToken::new(), None)
        .await
        .unwrap();

    assert!(!result.is_error);
    assert_eq!(result.data["taskId"], id);
    let output = result.data["output"].as_str().unwrap_or("");
    assert!(
        output.contains("output description"),
        "output should contain description text"
    );
}

#[test]
fn test_task_tool_properties() {
    let create = TaskCreateTool;
    let list = TaskListTool;
    let get = TaskGetTool;
    let update = TaskUpdateTool;
    let stop = TaskStopTool;
    let output = TaskOutputTool;

    let dummy = json!({});

    // Create: not safe, not read-only
    assert!(!create.is_concurrency_safe(&dummy));
    assert!(!create.is_read_only(&dummy));

    // List: safe, read-only
    assert!(list.is_concurrency_safe(&dummy));
    assert!(list.is_read_only(&dummy));

    // Get: safe, read-only
    assert!(get.is_concurrency_safe(&dummy));
    assert!(get.is_read_only(&dummy));

    // Update: not safe, not read-only
    assert!(!update.is_concurrency_safe(&dummy));
    assert!(!update.is_read_only(&dummy));

    // Stop: not safe, not read-only
    assert!(!stop.is_concurrency_safe(&dummy));
    assert!(!stop.is_read_only(&dummy));

    // Output: safe, read-only
    assert!(output.is_concurrency_safe(&dummy));
    assert!(output.is_read_only(&dummy));
}
