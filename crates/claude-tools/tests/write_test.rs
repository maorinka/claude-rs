use claude_core::types::events::ToolResultData;
use claude_tools::registry::{ToolExecutor, ToolUseContext};
use claude_tools::write::FileWriteTool;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

fn make_ctx(dir: &std::path::Path) -> ToolUseContext {
    ToolUseContext::for_test(
        dir.to_path_buf(),
        std::sync::Arc::new(std::sync::Mutex::new(
            claude_tools::registry::ReadFileState::new(),
        )),
        claude_tools::registry::PermissionMode::Default,
    )
}

async fn call_tool(tool: &FileWriteTool, input: Value, ctx: &ToolUseContext) -> ToolResultData {
    tool.call(&input, ctx, CancellationToken::new(), None)
        .await
        .expect("tool call should succeed")
}

#[tokio::test]
async fn test_write_new_file() {
    let tmp = tempfile::tempdir().unwrap();
    let tool = FileWriteTool;
    let ctx = make_ctx(tmp.path());
    let file_path = tmp
        .path()
        .join("new_file.txt")
        .to_string_lossy()
        .to_string();
    let content = "hello, world!";

    let result = call_tool(
        &tool,
        json!({ "file_path": file_path, "content": content }),
        &ctx,
    )
    .await;

    assert!(!result.is_error);
    assert_eq!(result.data["type"], "create");
    assert_eq!(result.data["filePath"], file_path);
    assert_eq!(result.data["content"], content);
    assert!(result.data["originalFile"].is_null());
    assert_eq!(result.data["structuredPatch"], json!([]));

    let on_disk = std::fs::read_to_string(&file_path).unwrap();
    assert_eq!(on_disk, content);
}

#[tokio::test]
async fn test_write_creates_parent_dirs() {
    let tmp = tempfile::tempdir().unwrap();
    let tool = FileWriteTool;
    let ctx = make_ctx(tmp.path());
    let file_path = tmp
        .path()
        .join("a/b/c/deep.txt")
        .to_string_lossy()
        .to_string();
    let content = "deep content";

    let result = call_tool(
        &tool,
        json!({ "file_path": file_path, "content": content }),
        &ctx,
    )
    .await;

    assert!(!result.is_error);
    assert_eq!(result.data["type"], "create");

    let on_disk = std::fs::read_to_string(&file_path).unwrap();
    assert_eq!(on_disk, content);
    assert!(tmp.path().join("a/b/c").is_dir());
}

#[tokio::test]
async fn test_write_expands_relative_path_against_context_cwd() {
    let tmp = tempfile::tempdir().unwrap();
    let tool = FileWriteTool;
    let ctx = make_ctx(tmp.path());

    let result = call_tool(
        &tool,
        json!({ "file_path": "relative.txt", "content": "relative content" }),
        &ctx,
    )
    .await;

    let expected_path = tmp.path().join("relative.txt");
    assert!(!result.is_error);
    assert_eq!(
        result.data["filePath"],
        expected_path.to_string_lossy().as_ref()
    );
    assert_eq!(
        std::fs::read_to_string(expected_path).unwrap(),
        "relative content"
    );
}

#[test]
fn test_write_permission_expands_relative_path_against_context_cwd() {
    let tmp = tempfile::tempdir().unwrap();
    let tool = FileWriteTool;
    let context = claude_core::permissions::ToolPermissionContext {
        mode: claude_core::permissions::PermissionMode::AcceptEdits,
        working_directory: tmp.path().to_path_buf(),
        ..Default::default()
    };

    let result = tool.check_permissions(
        &json!({ "file_path": "relative.txt", "content": "x" }),
        &context,
    );

    assert!(
        matches!(result, claude_core::permissions::PermissionResult::Allow(_)),
        "relative write inside cwd should be allowed in acceptEdits mode"
    );
}

#[tokio::test]
async fn test_write_overwrites_existing() {
    let tmp = tempfile::tempdir().unwrap();
    let tool = FileWriteTool;
    let ctx = make_ctx(tmp.path());
    let file_path = tmp
        .path()
        .join("existing.txt")
        .to_string_lossy()
        .to_string();
    let original = "original content";
    let new_content = "updated content";

    // Create the file first
    std::fs::write(&file_path, original).unwrap();

    // Record a read (required by staleness check before overwriting)
    ctx.read_file_state
        .lock()
        .unwrap()
        .record_read(&file_path, false, None);

    let result = call_tool(
        &tool,
        json!({ "file_path": file_path, "content": new_content }),
        &ctx,
    )
    .await;

    assert!(!result.is_error);
    assert_eq!(result.data["type"], "update");
    assert_eq!(result.data["content"], new_content);
    assert_eq!(result.data["originalFile"], original);
    let patch = result.data["structuredPatch"]
        .as_array()
        .expect("structuredPatch must be an array");
    assert!(!patch.is_empty(), "updates must include a structured diff");
    assert_eq!(patch[0]["oldStart"], 1);
    assert_eq!(patch[0]["newStart"], 1);
    assert!(patch[0]["lines"]
        .as_array()
        .unwrap()
        .iter()
        .any(|line| line == "-original content"));
    assert!(patch[0]["lines"]
        .as_array()
        .unwrap()
        .iter()
        .any(|line| line == "+updated content"));

    let on_disk = std::fs::read_to_string(&file_path).unwrap();
    assert_eq!(on_disk, new_content);

    let mtime_ms = std::fs::metadata(&file_path)
        .unwrap()
        .modified()
        .unwrap()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let state = ctx.read_file_state.lock().unwrap();
    assert_eq!(
        state.get(&file_path).unwrap().timestamp,
        mtime_ms,
        "write should store the file mtime like TS"
    );
}

#[tokio::test]
async fn test_write_is_destructive() {
    let tool = FileWriteTool;
    let input = json!({ "file_path": "/tmp/x.txt", "content": "y" });

    assert!(tool.is_destructive(&input));
    assert!(!tool.is_concurrency_safe(&input));
    assert!(!tool.is_read_only(&input));
    assert_eq!(tool.max_result_size_chars(), 100_000);
    assert_eq!(tool.name(), "Write");
}

#[tokio::test]
async fn test_write_empty_existing_file_matches_ts_create_result() {
    let tmp = tempfile::tempdir().unwrap();
    let tool = FileWriteTool;
    let ctx = make_ctx(tmp.path());
    let file_path = tmp.path().join("empty.txt").to_string_lossy().to_string();

    std::fs::write(&file_path, "").unwrap();
    ctx.read_file_state
        .lock()
        .unwrap()
        .record_read(&file_path, false, Some(String::new()));

    let result = call_tool(
        &tool,
        json!({ "file_path": file_path, "content": "now has content" }),
        &ctx,
    )
    .await;

    assert!(!result.is_error);
    assert_eq!(result.data["type"], "create");
    assert!(result.data["originalFile"].is_null());
    assert_eq!(result.data["structuredPatch"], json!([]));
}

// ─── team_mem_secret_guard integration ──────────────────────────────────────
//
// These tests exercise the guard wired into FileWriteTool::call. They
// share a mutex because they mutate the global TEAMMEM +
// CLAUDE_CONFIG_DIR env vars — cargo runs tests in parallel by default.

use std::sync::Mutex as StdMutex;
static GUARD_ENV_LOCK: StdMutex<()> = StdMutex::new(());

const AWS_KEY: &str = "AKIAIOSFODNN7EXAMPLE";

/// Scope helper that pins `CLAUDE_CONFIG_DIR` + `TEAMMEM` for the
/// duration of a test and cleans them up on drop. The memory base
/// becomes the supplied tempdir so team-mem paths are writable.
struct TeamMemEnv<'a> {
    _guard: std::sync::MutexGuard<'a, ()>,
}

impl<'a> TeamMemEnv<'a> {
    fn enter(config_dir: &std::path::Path) -> Self {
        let guard = GUARD_ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        std::env::set_var("CLAUDE_CONFIG_DIR", config_dir);
        std::env::set_var("TEAMMEM", "1");
        std::env::remove_var("CLAUDE_CODE_DISABLE_AUTO_MEMORY");
        TeamMemEnv { _guard: guard }
    }
}

impl Drop for TeamMemEnv<'_> {
    fn drop(&mut self) {
        std::env::remove_var("CLAUDE_CONFIG_DIR");
        std::env::remove_var("TEAMMEM");
    }
}

#[tokio::test]
async fn guard_blocks_secret_in_team_memory_path() {
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path().join(".claude");
    std::fs::create_dir_all(&config_dir).unwrap();
    let _env = TeamMemEnv::enter(&config_dir);

    // cwd is passed through `ctx.working_directory` — no need to
    // mutate the process-global cwd.
    let cwd = tmp.path().to_path_buf();
    let team_dir = claude_core::memdir::team_mem_paths::get_team_mem_path(&cwd);
    std::fs::create_dir_all(&team_dir).unwrap();

    let tool = FileWriteTool;
    let ctx = make_ctx(&cwd);
    let file_path = team_dir.join("leaked.md").to_string_lossy().to_string();

    let result = call_tool(
        &tool,
        json!({ "file_path": file_path, "content": format!("Team memory with {AWS_KEY}") }),
        &ctx,
    )
    .await;

    assert!(result.is_error, "guard must reject secret-containing write");
    let err = result.data["error"].as_str().unwrap();
    assert!(
        err.contains("Team memory is shared"),
        "error must come from team_mem_secret_guard, got: {err}"
    );
}

#[tokio::test]
async fn guard_allows_benign_team_memory_write() {
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path().join(".claude");
    std::fs::create_dir_all(&config_dir).unwrap();
    let _env = TeamMemEnv::enter(&config_dir);

    let cwd = tmp.path().to_path_buf();
    let team_dir = claude_core::memdir::team_mem_paths::get_team_mem_path(&cwd);
    std::fs::create_dir_all(&team_dir).unwrap();

    let tool = FileWriteTool;
    let ctx = make_ctx(&cwd);
    let file_path = team_dir.join("benign.md").to_string_lossy().to_string();

    let result = call_tool(
        &tool,
        json!({
            "file_path": file_path,
            "content": "Benign team memory — no keys here."
        }),
        &ctx,
    )
    .await;

    assert!(!result.is_error, "benign team-memory writes must succeed");
}

#[tokio::test]
async fn guard_skips_writes_outside_team_memory() {
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path().join(".claude");
    std::fs::create_dir_all(&config_dir).unwrap();
    let _env = TeamMemEnv::enter(&config_dir);

    let cwd = tmp.path().to_path_buf();
    let tool = FileWriteTool;
    let ctx = make_ctx(&cwd);
    let file_path = cwd.join("normal.md").to_string_lossy().to_string();

    let result = call_tool(
        &tool,
        json!({
            "file_path": file_path,
            "content": format!("Outside team mem — {AWS_KEY}")
        }),
        &ctx,
    )
    .await;

    assert!(!result.is_error, "non-team-path writes must skip the guard");
}

/// After a FileWrite, the stored `read_file_state.content` must be
/// the exact `content` argument (not LF-normalised) — matches TS
/// `FileWriteTool.ts:331` which stores the raw value passed to
/// `writeTextContent`. Staleness correctness is preserved because
/// `check_file_staleness` normalises both sides before comparing.
#[tokio::test]
async fn test_write_stores_raw_content_in_read_state() {
    let tmp = tempfile::tempdir().unwrap();
    let tool = FileWriteTool;
    let ctx = make_ctx(tmp.path());
    let file_path = tmp.path().join("stored.txt").to_string_lossy().to_string();
    let content = "a\r\nb\r\nc";

    let result = call_tool(
        &tool,
        json!({ "file_path": file_path, "content": content }),
        &ctx,
    )
    .await;
    assert!(!result.is_error);

    let state = ctx.read_file_state.lock().unwrap();
    let entry = state.get(&file_path).expect("read state must be populated");
    assert_eq!(
        entry.content.as_deref(),
        Some(content),
        "stored content must match raw `content` arg, not an LF-normalised copy"
    );
}

/// An empty-string write should store `Some("")` in the read-state,
/// not `None` — staleness behaviour must differentiate a write we
/// just performed (even if it wrote no bytes) from a never-seen file.
#[tokio::test]
async fn test_write_empty_content_stores_some_empty_string() {
    let tmp = tempfile::tempdir().unwrap();
    let tool = FileWriteTool;
    let ctx = make_ctx(tmp.path());
    let file_path = tmp.path().join("blank.txt").to_string_lossy().to_string();

    let result = call_tool(
        &tool,
        json!({ "file_path": file_path, "content": "" }),
        &ctx,
    )
    .await;
    assert!(!result.is_error);

    let state = ctx.read_file_state.lock().unwrap();
    let entry = state.get(&file_path).expect("entry must exist");
    assert_eq!(entry.content.as_deref(), Some(""));
    assert!(!entry.is_partial_view);
}
