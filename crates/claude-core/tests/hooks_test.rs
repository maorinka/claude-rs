use claude_core::hooks::runner::HookRunner;
use claude_core::hooks::types::{HookConfig, HookEvent};

#[tokio::test]
async fn test_hook_runner_echo_command() {
    let hooks = vec![HookConfig {
        event: HookEvent::PostResponse,
        command: "echo hello".to_string(),
        timeout_ms: None,
    }];
    let runner = HookRunner::new(hooks);
    let results = runner
        .run_hooks(HookEvent::PostResponse, &[])
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].exit_code, 0);
    assert!(results[0].stdout.contains("hello"));
}

#[tokio::test]
async fn test_hook_runner_wrong_event_not_run() {
    let hooks = vec![HookConfig {
        event: HookEvent::PreToolUse,
        command: "echo should_not_run".to_string(),
        timeout_ms: None,
    }];
    let runner = HookRunner::new(hooks);
    // Run with PostResponse — hook is for PreToolUse, should not execute
    let results = runner
        .run_hooks(HookEvent::PostResponse, &[])
        .await
        .unwrap();
    assert_eq!(results.len(), 0);
}

#[tokio::test]
async fn test_hook_runner_env_variable_passed() {
    let hooks = vec![HookConfig {
        event: HookEvent::SessionStart,
        command: "echo $TEST_VAR".to_string(),
        timeout_ms: None,
    }];
    let runner = HookRunner::new(hooks);
    let results = runner
        .run_hooks(HookEvent::SessionStart, &[("TEST_VAR", "hello_world")])
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert!(results[0].stdout.contains("hello_world"));
}

#[tokio::test]
async fn test_hook_runner_from_settings_empty() {
    let settings = serde_json::json!({});
    let runner = HookRunner::from_settings(&settings);
    let results = runner
        .run_hooks(HookEvent::PostResponse, &[])
        .await
        .unwrap();
    assert_eq!(results.len(), 0);
}

#[tokio::test]
async fn test_hook_runner_from_settings_with_hooks() {
    let settings = serde_json::json!({
        "hooks": [
            { "event": "post_response", "command": "echo from_settings" }
        ]
    });
    let runner = HookRunner::from_settings(&settings);
    let results = runner
        .run_hooks(HookEvent::PostResponse, &[])
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].exit_code, 0);
    assert!(results[0].stdout.contains("from_settings"));
}

#[tokio::test]
async fn test_hook_runner_nonzero_exit_code() {
    let hooks = vec![HookConfig {
        event: HookEvent::SessionEnd,
        command: "exit 42".to_string(),
        timeout_ms: None,
    }];
    let runner = HookRunner::new(hooks);
    let results = runner.run_hooks(HookEvent::SessionEnd, &[]).await.unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].exit_code, 42);
}
