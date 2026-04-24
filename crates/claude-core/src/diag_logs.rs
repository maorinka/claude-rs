//! Diagnostic logging — JSON-line append to an env-var-designated file.
//!
//! Port of TS `utils/diagLogs.ts:1-94`.
//!
//! Writes one JSON object per line to the path in
//! `CLAUDE_CODE_DIAGNOSTICS_FILE`. The env-manager sidecar tails the file
//! and ships entries to session-ingress so container-hosted failures can
//! be observed from outside.
//!
//! **No PII.** Callers must not pass file paths, project names, repo
//! names, prompts, or any user content — logs leave the container.

use chrono::Utc;
use serde::Serialize;
use serde_json::{Map, Value};
use std::future::Future;
use std::time::Instant;

/// TS declares these as a string union; Rust uses an enum with
/// lowercase serde tags to produce the same wire values.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosticLogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Serialize)]
struct DiagnosticLogEntry<'a> {
    timestamp: String,
    level: DiagnosticLogLevel,
    event: &'a str,
    /// Always emitted — TS writes `data ?? {}` so the field is present
    /// even when empty; the downstream shipper relies on it.
    data: Map<String, Value>,
}

/// Env var naming the diagnostics sink path. Matches TS
/// `process.env.CLAUDE_CODE_DIAGNOSTICS_FILE` verbatim.
const DIAGNOSTICS_ENV: &str = "CLAUDE_CODE_DIAGNOSTICS_FILE";

fn get_diagnostic_log_file() -> Option<std::path::PathBuf> {
    std::env::var_os(DIAGNOSTICS_ENV).map(Into::into)
}

/// Append one diagnostic event. Silent on every failure — TS wraps the
/// append in a two-level try/catch that creates parent dirs and then
/// swallows anything that still fails. Matching that exactly: diagnostic
/// logging is opt-in via env var, never a hard dependency of the caller.
///
/// **No PII in `event` or `data`.** See module docs.
pub fn log_for_diagnostics_no_pii(
    level: DiagnosticLogLevel,
    event: &str,
    data: Option<Map<String, Value>>,
) {
    let Some(log_file) = get_diagnostic_log_file() else {
        return;
    };

    let entry = DiagnosticLogEntry {
        timestamp: Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        level,
        event,
        data: data.unwrap_or_default(),
    };

    let Ok(mut line) = serde_json::to_string(&entry) else {
        return;
    };
    line.push('\n');

    if append_line(&log_file, &line).is_err() {
        // Parent dir might not exist yet — TS tries `mkdir` + retry once,
        // then swallows. Same here.
        if let Some(parent) = log_file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = append_line(&log_file, &line);
    }
}

fn append_line(path: &std::path::Path, line: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    f.write_all(line.as_bytes())
}

/// Time `fut` and emit three entries around it: `{event}_started`,
/// `{event}_completed` (on `Ok`), or `{event}_failed` (on `Err`).
/// Matches TS `withDiagnosticsTiming` — same event-name suffixes so
/// downstream queries work unchanged.
///
/// `get_data` pulls additional fields from the success result; on error
/// only `duration_ms` is logged (TS drops the data closure on the error
/// path too, see `diagLogs.ts:89-91`).
pub async fn with_diagnostics_timing<T, E, F, Fut, G>(
    event: &str,
    fut: F,
    get_data: Option<G>,
) -> Result<T, E>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    G: FnOnce(&T) -> Map<String, Value>,
{
    let start = Instant::now();
    log_for_diagnostics_no_pii(
        DiagnosticLogLevel::Info,
        &format!("{event}_started"),
        None,
    );

    match fut().await {
        Ok(result) => {
            let mut data = get_data.map(|g| g(&result)).unwrap_or_default();
            data.insert(
                "duration_ms".into(),
                Value::from(start.elapsed().as_millis() as u64),
            );
            log_for_diagnostics_no_pii(
                DiagnosticLogLevel::Info,
                &format!("{event}_completed"),
                Some(data),
            );
            Ok(result)
        }
        Err(err) => {
            let mut data = Map::new();
            data.insert(
                "duration_ms".into(),
                Value::from(start.elapsed().as_millis() as u64),
            );
            log_for_diagnostics_no_pii(
                DiagnosticLogLevel::Error,
                &format!("{event}_failed"),
                Some(data),
            );
            Err(err)
        }
    }
}

#[cfg(test)]
#[allow(clippy::await_holding_lock)] // test-only env serialization via std::sync::Mutex
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;

    // The diagnostics env var is process-global, so tests that set it must
    // serialise to avoid one test's teardown nuking another's setup.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn lock_env() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn set_diag_file(path: &std::path::Path) {
        std::env::set_var(DIAGNOSTICS_ENV, path);
    }

    fn clear_diag_file() {
        std::env::remove_var(DIAGNOSTICS_ENV);
    }

    fn read_events(path: &std::path::Path) -> Vec<Value> {
        std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    #[test]
    fn no_op_when_env_unset() {
        let _g = lock_env();
        clear_diag_file();
        // No panic, no side effect.
        log_for_diagnostics_no_pii(DiagnosticLogLevel::Info, "noop", None);
    }

    #[test]
    fn writes_jsonl_entry() {
        let _g = lock_env();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("diag.log");
        set_diag_file(&path);

        let mut data = Map::new();
        data.insert("k".into(), json!(1));
        log_for_diagnostics_no_pii(DiagnosticLogLevel::Info, "ev", Some(data));
        clear_diag_file();

        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.ends_with('\n'));
        let parsed: Value = serde_json::from_str(contents.trim()).unwrap();
        assert_eq!(parsed["level"], "info");
        assert_eq!(parsed["event"], "ev");
        assert_eq!(parsed["data"]["k"], 1);
        assert!(parsed["timestamp"].as_str().unwrap().contains('T'));
    }

    #[test]
    fn missing_data_serialises_as_empty_object() {
        let _g = lock_env();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("diag.log");
        set_diag_file(&path);

        log_for_diagnostics_no_pii(DiagnosticLogLevel::Warn, "ev", None);
        clear_diag_file();

        let line = std::fs::read_to_string(&path).unwrap();
        let parsed: Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(parsed["data"], json!({}));
        assert_eq!(parsed["level"], "warn");
    }

    #[test]
    fn appends_when_called_repeatedly() {
        let _g = lock_env();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("diag.log");
        set_diag_file(&path);

        log_for_diagnostics_no_pii(DiagnosticLogLevel::Info, "a", None);
        log_for_diagnostics_no_pii(DiagnosticLogLevel::Info, "b", None);
        clear_diag_file();

        let contents = std::fs::read_to_string(&path).unwrap();
        assert_eq!(contents.matches('\n').count(), 2);
    }

    #[test]
    fn creates_parent_dir_on_first_write() {
        let _g = lock_env();
        let dir = tempfile::tempdir().unwrap();
        // Target inside a subdir that doesn't exist yet.
        let path = dir.path().join("nested/missing/diag.log");
        set_diag_file(&path);

        log_for_diagnostics_no_pii(DiagnosticLogLevel::Info, "ev", None);
        clear_diag_file();

        assert!(path.exists(), "{path:?} should have been created");
    }

    #[test]
    fn all_levels_serialise_to_lowercase_tags() {
        // Pins the wire format — downstream consumers filter by exact string.
        for (lvl, tag) in [
            (DiagnosticLogLevel::Debug, "debug"),
            (DiagnosticLogLevel::Info, "info"),
            (DiagnosticLogLevel::Warn, "warn"),
            (DiagnosticLogLevel::Error, "error"),
        ] {
            let s = serde_json::to_string(&lvl).unwrap();
            assert_eq!(s, format!("\"{tag}\""));
        }
    }

    #[tokio::test]
    async fn timing_wrapper_logs_start_and_completed() {
        let _g = lock_env();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("diag.log");
        set_diag_file(&path);

        let out = with_diagnostics_timing::<_, (), _, _, fn(&i32) -> Map<String, Value>>(
            "work",
            || async { Ok::<_, ()>(42) },
            None,
        )
        .await;
        clear_diag_file();
        assert_eq!(out, Ok(42));

        let events = read_events(&path);
        let names: Vec<&str> = events.iter().map(|v| v["event"].as_str().unwrap()).collect();
        assert_eq!(names, vec!["work_started", "work_completed"]);
        assert!(events[1]["data"]["duration_ms"].is_number());
    }

    #[tokio::test]
    async fn timing_wrapper_logs_failed_on_err() {
        let _g = lock_env();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("diag.log");
        set_diag_file(&path);

        let result = with_diagnostics_timing::<i32, &str, _, _, fn(&i32) -> Map<String, Value>>(
            "work",
            || async { Err("boom") },
            None,
        )
        .await;
        clear_diag_file();
        assert_eq!(result, Err("boom"));

        let events = read_events(&path);
        let names: Vec<&str> = events.iter().map(|v| v["event"].as_str().unwrap()).collect();
        assert_eq!(names, vec!["work_started", "work_failed"]);
        assert_eq!(events[1]["level"], "error");
        assert!(events[1]["data"]["duration_ms"].is_number());
    }

    #[tokio::test]
    async fn timing_wrapper_merges_get_data_into_completed() {
        let _g = lock_env();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("diag.log");
        set_diag_file(&path);

        with_diagnostics_timing::<i32, (), _, _, _>(
            "work",
            || async { Ok::<_, ()>(7) },
            Some(|r: &i32| {
                let mut m = Map::new();
                m.insert("result".into(), Value::from(*r));
                m
            }),
        )
        .await
        .unwrap();
        clear_diag_file();

        let events = read_events(&path);
        let last = events.last().unwrap();
        assert_eq!(last["event"], "work_completed");
        assert_eq!(last["data"]["result"], 7);
        assert!(last["data"]["duration_ms"].is_number());
    }
}
