use claude_core::context::environment::*;
use claude_core::context::system_prompt::*;

#[test]
fn test_environment_context_contains_platform() {
    let ctx = build_environment_context();
    assert!(ctx.contains("Platform:"));
    assert!(ctx.contains(std::env::consts::OS));
}

#[test]
fn test_environment_context_contains_cwd() {
    let ctx = build_environment_context();
    assert!(ctx.contains("Working directory:"));
}

#[tokio::test]
async fn test_build_system_prompt_basic() {
    let tmp = tempfile::tempdir().unwrap();
    let tools = vec![
        ("Read".into(), "Read files".into()),
        ("Bash".into(), "Run commands".into()),
    ];
    let blocks = build_system_prompt(tmp.path(), &tools, "claude-sonnet-4-6")
        .await
        .unwrap();
    assert!(blocks.len() >= 2); // base + environment (git may or may not be present)

    // Check base prompt is first
    let first_text = blocks[0]["text"].as_str().unwrap();
    assert!(first_text.contains("Claude"));
}

#[tokio::test]
async fn test_build_system_prompt_omits_duplicate_tools() {
    let tmp = tempfile::tempdir().unwrap();
    let tools = vec![(
        "ParityOnlyToolName".into(),
        "Parity-only tool description".into(),
    )];
    let blocks = build_system_prompt(tmp.path(), &tools, "claude-sonnet-4-6")
        .await
        .unwrap();

    let all_text: String = blocks
        .iter()
        .filter_map(|b| b["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!all_text.contains("ParityOnlyToolName"));
    assert!(!all_text.contains("Parity-only tool description"));
}

#[tokio::test]
async fn test_git_context_in_non_git_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let git_ctx = claude_core::context::git::get_git_context(tmp.path())
        .await
        .unwrap();
    assert!(git_ctx.is_none());
}
