use claude_core::query::tool_executor::*;
use claude_core::types::events::ToolResultData;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

fn make_call_fn() -> ToolCallFn {
    Arc::new(|name, id, _input, _cancel| {
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

fn make_delayed_call_fn() -> ToolCallFn {
    Arc::new(|name, id, input, _cancel| {
        tokio::spawn(async move {
            let delay = input
                .get("delay_ms")
                .and_then(|value| value.as_u64())
                .unwrap_or(10);
            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
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

#[tokio::test]
async fn test_results_yield_in_tool_use_order() {
    let cancel = CancellationToken::new();
    let mut exec = StreamingToolExecutor::new(cancel, make_delayed_call_fn());

    exec.add_tool(PendingTool {
        id: "tu_slow".into(),
        name: "Read".into(),
        input: serde_json::json!({"delay_ms": 50}),
        is_concurrent: true,
    });
    exec.add_tool(PendingTool {
        id: "tu_fast".into(),
        name: "Glob".into(),
        input: serde_json::json!({"delay_ms": 1}),
        is_concurrent: true,
    });

    let results = exec.flush().await;
    let ids = results
        .into_iter()
        .map(|result| result.id)
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["tu_slow", "tu_fast"]);
}

#[tokio::test]
async fn test_exclusive_tool_blocks_later_concurrent_tools() {
    let cancel = CancellationToken::new();
    let mut exec = StreamingToolExecutor::new(cancel, make_delayed_call_fn());

    exec.add_tool(PendingTool {
        id: "tu_read_1".into(),
        name: "Read".into(),
        input: serde_json::json!({"delay_ms": 50}),
        is_concurrent: true,
    });
    exec.add_tool(PendingTool {
        id: "tu_bash".into(),
        name: "Bash".into(),
        input: serde_json::json!({"delay_ms": 1}),
        is_concurrent: false,
    });
    exec.add_tool(PendingTool {
        id: "tu_read_2".into(),
        name: "Read".into(),
        input: serde_json::json!({"delay_ms": 1}),
        is_concurrent: true,
    });

    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    assert!(
        exec.poll_completed().is_empty(),
        "later concurrent tool must not run ahead of queued exclusive tool"
    );

    let results = exec.flush().await;
    let ids = results
        .into_iter()
        .map(|result| result.id)
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["tu_read_1", "tu_bash", "tu_read_2"]);
}
