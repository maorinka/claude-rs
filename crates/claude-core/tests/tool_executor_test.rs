use claude_core::query::tool_executor::*;
use claude_core::types::events::ToolResultData;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

fn make_call_fn() -> ToolCallFn {
    Arc::new(|name, id, input, cancel| {
        tokio::spawn(async move {
            // Simulate tool execution
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            Ok(ToolResultData {
                data: serde_json::json!({"tool": name, "id": id}),
                is_error: false,
            })
        })
    })
}

#[tokio::test]
async fn test_execute_single_tool() {
    let cancel = CancellationToken::new();
    let mut exec = StreamingToolExecutor::new(cancel, make_call_fn());

    exec.add_tool(PendingTool {
        id: "tu_1".into(),
        name: "Read".into(),
        input: serde_json::json!({}),
        is_concurrent: true,
    });

    let results = exec.flush().await;
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "tu_1");
    assert!(results[0].result.is_ok());
}

#[tokio::test]
async fn test_concurrent_tools_run_parallel() {
    let cancel = CancellationToken::new();
    let mut exec = StreamingToolExecutor::new(cancel, make_call_fn());

    exec.add_tool(PendingTool {
        id: "tu_1".into(),
        name: "Read".into(),
        input: serde_json::json!({}),
        is_concurrent: true,
    });
    exec.add_tool(PendingTool {
        id: "tu_2".into(),
        name: "Glob".into(),
        input: serde_json::json!({}),
        is_concurrent: true,
    });
    exec.add_tool(PendingTool {
        id: "tu_3".into(),
        name: "Grep".into(),
        input: serde_json::json!({}),
        is_concurrent: true,
    });

    let results = exec.flush().await;
    assert_eq!(results.len(), 3);
}

#[tokio::test]
async fn test_exclusive_tool_queues() {
    let cancel = CancellationToken::new();
    let mut exec = StreamingToolExecutor::new(cancel, make_call_fn());

    exec.add_tool(PendingTool {
        id: "tu_1".into(),
        name: "Read".into(),
        input: serde_json::json!({}),
        is_concurrent: true,
    });
    exec.add_tool(PendingTool {
        id: "tu_2".into(),
        name: "Bash".into(),
        input: serde_json::json!({}),
        is_concurrent: false,
    });
    exec.add_tool(PendingTool {
        id: "tu_3".into(),
        name: "Read".into(),
        input: serde_json::json!({}),
        is_concurrent: true,
    });

    let results = exec.flush().await;
    assert_eq!(results.len(), 3);
}

#[tokio::test]
async fn test_has_pending() {
    let cancel = CancellationToken::new();
    let mut exec = StreamingToolExecutor::new(cancel, make_call_fn());
    assert!(!exec.has_pending());

    exec.add_tool(PendingTool {
        id: "tu_1".into(),
        name: "Read".into(),
        input: serde_json::json!({}),
        is_concurrent: true,
    });
    assert!(exec.has_pending());

    exec.flush().await;
    assert!(!exec.has_pending());
}
