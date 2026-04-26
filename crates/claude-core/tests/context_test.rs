use claude_core::context::environment::*;
use claude_core::context::system_prompt::*;
use std::process::Command;

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

#[tokio::test]
async fn test_git_context_prefers_origin_head_default_branch() {
    let tmp = tempfile::tempdir().unwrap();
    run_git(tmp.path(), &["init"]);
    run_git(tmp.path(), &["checkout", "-b", "feature"]);
    run_git(
        tmp.path(),
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/trunk",
        ],
    );

    let git_ctx = claude_core::context::git::get_git_context(tmp.path())
        .await
        .unwrap()
        .unwrap();

    assert!(git_ctx.contains("Current branch: feature"));
    assert!(git_ctx.contains("Main branch (you will usually use this for PRs): trunk"));
}

#[tokio::test]
async fn test_git_context_includes_user_and_truncates_status() {
    let tmp = tempfile::tempdir().unwrap();
    run_git(tmp.path(), &["init"]);
    run_git(tmp.path(), &["config", "user.name", "Test User"]);

    for i in 0..180 {
        std::fs::write(tmp.path().join(format!("very-long-file-name-{i}.txt")), "x").unwrap();
    }

    let git_ctx = claude_core::context::git::get_git_context(tmp.path())
        .await
        .unwrap()
        .unwrap();

    assert!(git_ctx.contains("Git user: Test User"));
    assert!(git_ctx.contains("... (truncated because it exceeds 2k characters."));
}

fn run_git(dir: &std::path::Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git command should run");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}
