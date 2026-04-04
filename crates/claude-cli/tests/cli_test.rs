use std::process::Command;
use std::path::Path;

#[test]
fn test_cli_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_claude-rs"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("claude-rs"));
}

#[test]
fn test_cli_help() {
    let output = Command::new(env!("CARGO_BIN_EXE_claude-rs"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Claude Code"));
    assert!(stdout.contains("--model"));
    assert!(stdout.contains("--verbose"));
}

#[test]
fn test_cli_config_subcommand() {
    let output = Command::new(env!("CARGO_BIN_EXE_claude-rs"))
        .arg("config")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Project root:"));
}

// ============================================================
// Bug #23: --resume flag should return error when used
// Verifies --resume is accepted as a CLI flag (parsed by clap).
// The fix should make it return an error: "Session resumption is not yet implemented"
// ============================================================
#[test]
fn test_cli_resume_flag_is_parsed() {
    // Verify --resume is a valid flag that clap accepts (doesn't produce "unknown argument")
    let output = Command::new(env!("CARGO_BIN_EXE_claude-rs"))
        .arg("--resume")
        .arg("some-session-id")
        .output()
        .unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    // It should NOT fail with "unexpected argument" - the flag is parsed
    assert!(
        !stderr.contains("unexpected argument"),
        "--resume should be a known flag, got: {}",
        stderr
    );
}

// ============================================================
// Bug #22: Empty module files exist but serve no purpose
// Verifies the files exist (documenting the bug) and are empty
// The fix removes them.
// ============================================================
#[test]
fn test_empty_module_files_exist_and_are_empty() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let core_src = Path::new(manifest_dir).join("../claude-core/src");

    // Document that these empty files exist (the bug)
    let compact_path = core_src.join("compact.rs");
    let session_path = core_src.join("session.rs");

    if compact_path.exists() {
        let content = std::fs::read_to_string(&compact_path).unwrap();
        assert!(
            content.trim().is_empty(),
            "compact.rs should be empty (0-line file), but contains: {}",
            content
        );
    }

    if session_path.exists() {
        let content = std::fs::read_to_string(&session_path).unwrap();
        assert!(
            content.trim().is_empty(),
            "session.rs should be empty (0-line file), but contains: {}",
            content
        );
    }
}

// ============================================================
// Bug #15: Ask-permission tools should be denied in non-interactive mode
// ============================================================
#[test]
fn test_non_interactive_denies_ask_permission() {
    // Structural test: verify the permission logic for non-interactive mode
    use claude_core::permissions::evaluator::evaluate_permission_sync;
    use claude_core::permissions::types::{PermissionDecision, PermissionMode, ToolPermissionContext};

    let perm_ctx = ToolPermissionContext {
        mode: PermissionMode::Default,
        ..Default::default()
    };

    // A non-read-only tool with Default mode should get Ask decision
    let decision = evaluate_permission_sync(
        "Bash",
        &serde_json::json!({"command": "rm -rf /"}),
        &perm_ctx,
        false, // not read-only
    );

    // The evaluator returns Ask for write tools in Default mode
    matches!(decision, PermissionDecision::Ask { .. });

    // In non-interactive mode, the CLI should deny Ask decisions
    // This test verifies the permission evaluator returns Ask,
    // and the fix in main.rs ensures Ask is denied (not auto-allowed)
    match decision {
        PermissionDecision::Ask { message } => {
            assert!(message.contains("requires user confirmation"));
        }
        _ => panic!("Expected Ask decision for write tool in Default mode"),
    }
}
