use claude_core::types::events::ToolResultData;
use claude_tools::edit::FileEditTool;
use claude_tools::registry::{ToolExecutor, ToolUseContext};
use serde_json::json;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

fn make_ctx(dir: &TempDir) -> ToolUseContext {
    ToolUseContext {
        working_directory: dir.path().to_path_buf(),
        read_file_state: std::sync::Arc::new(std::sync::Mutex::new(
            claude_tools::registry::ReadFileState::new(),
        )),
        permission_mode: claude_tools::registry::PermissionMode::Default,
        ..Default::default()
    }
}

async fn call_tool(
    tool: &FileEditTool,
    input: serde_json::Value,
    ctx: &ToolUseContext,
) -> ToolResultData {
    let cancel = CancellationToken::new();
    tool.call(&input, ctx, cancel, None).await.unwrap()
}

#[tokio::test]
async fn test_edit_replace_string() {
    let dir = TempDir::new().unwrap();
    let file_path = dir.path().join("hello.txt");
    std::fs::write(&file_path, "hello world").unwrap();

    let tool = FileEditTool;
    let ctx = make_ctx(&dir);

    // Simulate reading the file first (required by staleness check)
    ctx.read_file_state
        .lock()
        .unwrap()
        .record_read(file_path.to_str().unwrap(), false, None);

    let result = call_tool(
        &tool,
        json!({
            "file_path": file_path.to_str().unwrap(),
            "old_string": "hello",
            "new_string": "goodbye"
        }),
        &ctx,
    )
    .await;

    assert!(
        !result.is_error,
        "Expected success, got error: {:?}",
        result.data
    );

    let content = std::fs::read_to_string(&file_path).unwrap();
    assert_eq!(content, "goodbye world");

    let data = &result.data;
    assert_eq!(data["filePath"], file_path.to_str().unwrap());
    assert_eq!(data["oldString"], "hello");
    assert_eq!(data["newString"], "goodbye");
    assert!(!data["replaceAll"].as_bool().unwrap_or(true));
}

#[tokio::test]
async fn test_edit_replace_all() {
    let dir = TempDir::new().unwrap();
    let file_path = dir.path().join("foos.txt");
    std::fs::write(&file_path, "foo bar foo baz foo").unwrap();

    let tool = FileEditTool;
    let ctx = make_ctx(&dir);

    // Simulate reading the file first (required by staleness check)
    ctx.read_file_state
        .lock()
        .unwrap()
        .record_read(file_path.to_str().unwrap(), false, None);

    let result = call_tool(
        &tool,
        json!({
            "file_path": file_path.to_str().unwrap(),
            "old_string": "foo",
            "new_string": "qux",
            "replace_all": true
        }),
        &ctx,
    )
    .await;

    assert!(
        !result.is_error,
        "Expected success, got error: {:?}",
        result.data
    );

    let content = std::fs::read_to_string(&file_path).unwrap();
    assert_eq!(content, "qux bar qux baz qux");

    assert!(result.data["replaceAll"].as_bool().unwrap_or(false));
}

#[tokio::test]
async fn test_edit_error_on_ambiguous_match() {
    let dir = TempDir::new().unwrap();
    let file_path = dir.path().join("multi.txt");
    std::fs::write(&file_path, "foo bar foo").unwrap();

    let tool = FileEditTool;
    let ctx = make_ctx(&dir);

    // Simulate reading the file first (required by staleness check)
    ctx.read_file_state
        .lock()
        .unwrap()
        .record_read(file_path.to_str().unwrap(), false, None);

    let result = call_tool(
        &tool,
        json!({
            "file_path": file_path.to_str().unwrap(),
            "old_string": "foo",
            "new_string": "baz",
            "replace_all": false
        }),
        &ctx,
    )
    .await;

    assert!(result.is_error, "Expected error for ambiguous match");
}

#[tokio::test]
async fn test_edit_string_not_found() {
    let dir = TempDir::new().unwrap();
    let file_path = dir.path().join("notfound.txt");
    std::fs::write(&file_path, "hello world").unwrap();

    let tool = FileEditTool;
    let ctx = make_ctx(&dir);

    // Simulate reading the file first (required by staleness check)
    ctx.read_file_state
        .lock()
        .unwrap()
        .record_read(file_path.to_str().unwrap(), false, None);

    let result = call_tool(
        &tool,
        json!({
            "file_path": file_path.to_str().unwrap(),
            "old_string": "missing string",
            "new_string": "replacement"
        }),
        &ctx,
    )
    .await;

    assert!(result.is_error, "Expected error when old_string not found");
}

#[tokio::test]
async fn test_edit_nonexistent_file() {
    let dir = TempDir::new().unwrap();
    let file_path = dir.path().join("does_not_exist.txt");

    let tool = FileEditTool;
    let ctx = make_ctx(&dir);

    let result = call_tool(
        &tool,
        json!({
            "file_path": file_path.to_str().unwrap(),
            "old_string": "hello",
            "new_string": "goodbye"
        }),
        &ctx,
    )
    .await;

    assert!(result.is_error, "Expected error for nonexistent file");
}

#[test]
fn test_edit_is_destructive() {
    let tool = FileEditTool;
    let input = json!({
        "file_path": "/some/path",
        "old_string": "a",
        "new_string": "b"
    });

    assert!(tool.is_destructive(&input));
    assert!(!tool.is_concurrency_safe(&input));
    assert!(!tool.is_read_only(&input));
    assert_eq!(tool.max_result_size_chars(), 100_000);
    assert_eq!(tool.name(), "Edit");
}

// ─── CRLF preservation ──────────────────────────────────────────────────────

/// A CRLF-formatted file should: (a) match `old_string` sent with LF
/// endings (the model's default), (b) land back on disk with its
/// original CRLF line endings preserved. Matches TS FileEditTool
/// behaviour at `FileEditTool.ts:214` + `:491`.
#[tokio::test]
async fn test_edit_preserves_crlf_line_endings() {
    let dir = TempDir::new().unwrap();
    let file_path = dir.path().join("windowsy.txt");
    let original_crlf = "first\r\nold line\r\nlast\r\n";
    std::fs::write(&file_path, original_crlf).unwrap();

    let tool = FileEditTool;
    let ctx = make_ctx(&dir);
    ctx.read_file_state.lock().unwrap().record_read(
        file_path.to_str().unwrap(),
        false,
        None,
    );

    // Model sends LF-normalised old_string + new_string.
    let result = call_tool(
        &tool,
        json!({
            "file_path": file_path.to_str().unwrap(),
            "old_string": "old line",
            "new_string": "new line",
        }),
        &ctx,
    )
    .await;

    assert!(!result.is_error, "edit should succeed despite CRLF on disk");

    // On-disk content must still use CRLF endings.
    let on_disk = std::fs::read(&file_path).unwrap();
    let as_str = String::from_utf8_lossy(&on_disk);
    assert_eq!(as_str, "first\r\nnew line\r\nlast\r\n");
}

/// LF-only files stay LF-only after an edit — we don't accidentally
/// promote a Unix file to CRLF.
#[tokio::test]
async fn test_edit_preserves_lf_line_endings() {
    let dir = TempDir::new().unwrap();
    let file_path = dir.path().join("unixy.txt");
    let original_lf = "first\nold line\nlast\n";
    std::fs::write(&file_path, original_lf).unwrap();

    let tool = FileEditTool;
    let ctx = make_ctx(&dir);
    ctx.read_file_state.lock().unwrap().record_read(
        file_path.to_str().unwrap(),
        false,
        None,
    );

    let result = call_tool(
        &tool,
        json!({
            "file_path": file_path.to_str().unwrap(),
            "old_string": "old line",
            "new_string": "new line",
        }),
        &ctx,
    )
    .await;

    assert!(!result.is_error);
    let on_disk = std::fs::read(&file_path).unwrap();
    let as_str = String::from_utf8_lossy(&on_disk);
    assert_eq!(as_str, "first\nnew line\nlast\n");
    assert!(
        !as_str.contains('\r'),
        "LF file must stay LF after edit — no accidental CRLF promotion"
    );
}

/// A `new_string` that itself contains CRLF (model pasted Windows
/// content into an edit) must not double-normalise into CRCRLF when
/// written back to a CRLF file. TS guards this at `file.ts:90-94`.
#[tokio::test]
async fn test_edit_new_string_with_crlf_does_not_double_normalize() {
    let dir = TempDir::new().unwrap();
    let file_path = dir.path().join("paste.txt");
    let original_crlf = "a\r\nb\r\n";
    std::fs::write(&file_path, original_crlf).unwrap();

    let tool = FileEditTool;
    let ctx = make_ctx(&dir);
    ctx.read_file_state.lock().unwrap().record_read(
        file_path.to_str().unwrap(),
        false,
        None,
    );

    // new_string already contains CRLF.
    let result = call_tool(
        &tool,
        json!({
            "file_path": file_path.to_str().unwrap(),
            "old_string": "b",
            "new_string": "x\r\ny",
        }),
        &ctx,
    )
    .await;

    assert!(!result.is_error);
    let on_disk = std::fs::read(&file_path).unwrap();
    let as_str = String::from_utf8_lossy(&on_disk);
    assert_eq!(
        as_str, "a\r\nx\r\ny\r\n",
        "CRLF should not double up (no CRCRLF)"
    );
    assert!(
        !as_str.contains("\r\r"),
        "sanity: no double-CR in output"
    );
}

/// The `originalFile` field in the tool result must report the
/// LF-normalised form so downstream consumers (model, diff
/// renderers) see the same text they'd see after a Read. Matches
/// TS which operates on the LF-normalised buffer throughout.
#[tokio::test]
async fn test_edit_originalfile_is_lf_normalised_for_crlf_disk() {
    let dir = TempDir::new().unwrap();
    let file_path = dir.path().join("crlf_result.txt");
    let original_crlf = "first\r\nold\r\nlast\r\n";
    std::fs::write(&file_path, original_crlf).unwrap();

    let tool = FileEditTool;
    let ctx = make_ctx(&dir);
    ctx.read_file_state.lock().unwrap().record_read(
        file_path.to_str().unwrap(),
        false,
        None,
    );

    let result = call_tool(
        &tool,
        json!({
            "file_path": file_path.to_str().unwrap(),
            "old_string": "old",
            "new_string": "new",
        }),
        &ctx,
    )
    .await;

    assert!(!result.is_error);
    let reported = result.data["originalFile"].as_str().unwrap();
    assert_eq!(
        reported, "first\nold\nlast\n",
        "originalFile must be LF-normalised even when disk is CRLF"
    );
    assert!(!reported.contains('\r'), "no CR should leak to the model");
}

/// After a successful edit, a harmless mtime bump (antivirus,
/// cloud-sync, `touch`) on the file must not cause the next edit
/// to be rejected as stale. The staleness check falls back to
/// content comparison only when `update_after_write` stored the
/// post-edit content. Regression for the codex CR finding that
/// `update_after_write` stored `content: None`, breaking this
/// fallback. TS parity: `FileEditTool.ts:520-525`.
#[tokio::test]
async fn test_edit_survives_mtime_touch_after_successful_edit() {
    let dir = TempDir::new().unwrap();
    let file_path = dir.path().join("touched.txt");
    std::fs::write(&file_path, "alpha\nbeta\ngamma\n").unwrap();

    let tool = FileEditTool;
    let ctx = make_ctx(&dir);
    ctx.read_file_state.lock().unwrap().record_read(
        file_path.to_str().unwrap(),
        false,
        Some("alpha\nbeta\ngamma\n".to_string()),
    );

    // First edit — populates read-state content via update_after_write.
    let first = call_tool(
        &tool,
        json!({
            "file_path": file_path.to_str().unwrap(),
            "old_string": "beta",
            "new_string": "BETA",
        }),
        &ctx,
    )
    .await;
    assert!(!first.is_error, "first edit should succeed");

    // Simulate an external tool bumping mtime without changing content
    // (antivirus scan, cloud-sync metadata touch). Sleep past the
    // millisecond floor so the mtime strictly exceeds the stored
    // read timestamp — otherwise the mtime-check short-circuits
    // before the content-comparison fallback is even exercised.
    std::thread::sleep(std::time::Duration::from_millis(20));
    let current = std::fs::read_to_string(&file_path).unwrap();
    std::fs::write(&file_path, &current).unwrap();

    // Second edit should still succeed: the content-comparison
    // fallback in check_file_staleness sees that disk == stored
    // post-edit content, so the mtime bump is tolerated.
    let second = call_tool(
        &tool,
        json!({
            "file_path": file_path.to_str().unwrap(),
            "old_string": "gamma",
            "new_string": "GAMMA",
        }),
        &ctx,
    )
    .await;
    assert!(
        !second.is_error,
        "second edit must not be rejected after a content-preserving \
         mtime touch — got: {:?}",
        second.data
    );

    let on_disk = std::fs::read_to_string(&file_path).unwrap();
    assert_eq!(on_disk, "alpha\nBETA\nGAMMA\n");
}

// ─── team_mem_secret_guard integration ──────────────────────────────────────

use std::sync::Mutex as StdMutex;
static GUARD_ENV_LOCK: StdMutex<()> = StdMutex::new(());

const AWS_KEY: &str = "AKIAIOSFODNN7EXAMPLE";

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

/// Regression for codex CR finding 1: the `!path.exists() &&
/// old_string.is_empty()` branch in FileEditTool wrote the new
/// file directly without consulting the guard. A FileEdit that
/// creates a brand-new team-memory file with a secret in
/// `new_string` must be rejected.
#[tokio::test]
async fn edit_guard_blocks_new_team_memory_file_with_secret() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join(".claude");
    std::fs::create_dir_all(&config_dir).unwrap();
    let _env = TeamMemEnv::enter(&config_dir);

    let cwd = tmp.path().to_path_buf();
    let team_dir = claude_core::memdir::team_mem_paths::get_team_mem_path(&cwd);
    std::fs::create_dir_all(&team_dir).unwrap();

    let ctx = ToolUseContext {
        working_directory: cwd.clone(),
        read_file_state: std::sync::Arc::new(std::sync::Mutex::new(
            claude_tools::registry::ReadFileState::new(),
        )),
        permission_mode: claude_tools::registry::PermissionMode::Default,
        ..Default::default()
    };
    let tool = FileEditTool;
    let file_path = team_dir
        .join("brand_new.md")
        .to_string_lossy()
        .to_string();

    let result = call_tool(
        &tool,
        json!({
            "file_path": file_path,
            "old_string": "",
            "new_string": format!("leaked: {AWS_KEY}")
        }),
        &ctx,
    )
    .await;

    assert!(result.is_error, "new-file Edit with secret must be rejected");
    assert!(result.data["error"]
        .as_str()
        .unwrap()
        .contains("Team memory is shared"));

    // Side-effect guard: the file must NOT exist — rejection
    // happens before the write.
    assert!(
        !std::path::Path::new(&file_path).exists(),
        "rejected edit must not leave the file on disk"
    );
}

/// Edits to an existing team-memory file: a `new_string` with a
/// secret must be rejected, matching TS `FileEditTool.ts:144`
/// which scans `new_string` only (not the projected post-edit
/// buffer).
#[tokio::test]
async fn edit_guard_blocks_existing_team_memory_file_with_secret() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join(".claude");
    std::fs::create_dir_all(&config_dir).unwrap();
    let _env = TeamMemEnv::enter(&config_dir);

    let cwd = tmp.path().to_path_buf();
    let team_dir = claude_core::memdir::team_mem_paths::get_team_mem_path(&cwd);
    std::fs::create_dir_all(&team_dir).unwrap();

    let file_path_buf = team_dir.join("existing.md");
    std::fs::write(&file_path_buf, "original\n").unwrap();
    let file_path = file_path_buf.to_string_lossy().to_string();

    let ctx = ToolUseContext {
        working_directory: cwd.clone(),
        read_file_state: std::sync::Arc::new(std::sync::Mutex::new(
            claude_tools::registry::ReadFileState::new(),
        )),
        permission_mode: claude_tools::registry::PermissionMode::Default,
        ..Default::default()
    };
    // Record a read so the staleness check passes — but the guard
    // should fire BEFORE staleness anyway per TS ordering.
    ctx.read_file_state
        .lock()
        .unwrap()
        .record_read(&file_path, false, None);

    let tool = FileEditTool;

    let result = call_tool(
        &tool,
        json!({
            "file_path": file_path,
            "old_string": "original",
            "new_string": format!("swapped for: {AWS_KEY}")
        }),
        &ctx,
    )
    .await;

    assert!(
        result.is_error,
        "edit that inserts a secret into team memory must be rejected"
    );
    assert!(result.data["error"]
        .as_str()
        .unwrap()
        .contains("Team memory is shared"));

    // The file should still have its original content.
    let on_disk = std::fs::read_to_string(&file_path_buf).unwrap();
    assert_eq!(on_disk, "original\n", "rejected edit must not touch disk");
}
