//! Cross-cutting error / env / sleep helpers.
//!
//! Port of the Rust-portable pieces of src/utils/errors.ts,
//! src/utils/envUtils.ts, and src/utils/sleep.ts. Some TS helpers
//! are web-API specific (AbortSignal / AbortController / APIUserAbortError)
//! and map to `tokio_util::sync::CancellationToken` on the Rust side,
//! so their signatures differ. The decision logic they wrap is
//! identical.

use std::io;
use std::time::Duration;

use thiserror::Error;
use tokio_util::sync::CancellationToken;

// ── Error types ──────────────────────────────────────────────────────────

/// Base claude-side error with a user-facing message. Most callers
/// convert from io / reqwest / anyhow errors rather than constructing
/// this directly.
#[derive(Debug, Error)]
#[error("{0}")]
pub struct ClaudeError(pub String);

#[derive(Debug, Error)]
#[error("aborted")]
pub struct AbortError;

#[derive(Debug, Error)]
#[error("malformed command")]
pub struct MalformedCommandError;

/// Shell-command failure with captured streams + exit code.
#[derive(Debug, Error)]
#[error("Shell command failed: exit code {code}")]
pub struct ShellError {
    pub stdout: String,
    pub stderr: String,
    pub code: i32,
    pub interrupted: bool,
}

/// Settings / config parse failure carrying the failed path and the
/// default that callers should fall back to. Matches TS
/// ConfigParseError.
#[derive(Debug, Error)]
#[error("Failed to parse {file_path}: {message}")]
pub struct ConfigParseError<T> {
    pub message: String,
    pub file_path: String,
    pub default_config: T,
}

// ── Message helpers ──────────────────────────────────────────────────────

/// Extract a display string from any error. Simpler than TS
/// `errorMessage` because Rust's Display trait handles this uniformly.
pub fn error_message<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// Keep only the first `max_frames` of a backtrace + the message
/// header. Use when surfacing errors to the model — full traces are
/// mostly-irrelevant internal frames that burn context tokens.
///
/// Port of TS `shortErrorStack`. Rust's panic-mode backtraces use a
/// different format (`<number>: <frame>`) but the "cap at N lines"
/// semantics are the same.
pub fn short_error_stack(e: &dyn std::error::Error, max_frames: usize) -> String {
    let msg = e.to_string();
    // std::error::Error + backtrace support is gated — approximate via
    // display chain. Walk the `source()` chain and include up to
    // max_frames causes.
    let mut frames = Vec::new();
    let mut cur: Option<&dyn std::error::Error> = e.source();
    while let Some(src) = cur {
        frames.push(format!("  caused by: {}", src));
        if frames.len() >= max_frames {
            break;
        }
        cur = src.source();
    }
    if frames.is_empty() {
        msg
    } else {
        format!("{}\n{}", msg, frames.join("\n"))
    }
}

// ── io::Error helpers ────────────────────────────────────────────────────

/// True iff `e` reports a "file or directory does not exist" error.
pub fn is_enoent(e: &io::Error) -> bool {
    matches!(e.kind(), io::ErrorKind::NotFound)
}

/// True iff the error means the path is unreachable for any of the
/// usual "expected missing / unreachable" reasons: not found,
/// permission denied, not a directory, or too many symlink levels.
/// Matches the set TS `isFsInaccessible` guards.
///
/// We check `ErrorKind` for the stable kinds and fall through to
/// `raw_os_error()` for `ENOTDIR` (20 macOS / 20 Linux) and `ELOOP`
/// (62 macOS / 40 Linux) which are still behind `#![feature(io_error_more)]`
/// at the time of this port.
pub fn is_fs_inaccessible(e: &io::Error) -> bool {
    use io::ErrorKind::*;
    if matches!(e.kind(), NotFound | PermissionDenied) {
        return true;
    }
    // Also catch the permission-adjacent codes that aren't modeled by
    // stable ErrorKind variants.
    match e.raw_os_error() {
        // EPERM
        Some(1) => true,
        // ENOTDIR: 20 on macOS/Linux
        Some(20) => true,
        // ELOOP: 62 on macOS, 40 on Linux
        Some(40) | Some(62) => true,
        _ => false,
    }
}

// ── Env helpers ──────────────────────────────────────────────────────────

/// Check whether an env var is set to a truthy value. Accepts
/// `1 / true / yes / on` case-insensitively. Matches TS `isEnvTruthy`.
pub fn is_env_truthy(name: &str) -> bool {
    match std::env::var(name) {
        Ok(v) => matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => false,
    }
}

/// Check whether an env var is explicitly set to a falsy value.
/// `0 / false / no / off` — case-insensitive. Differs from
/// `!is_env_truthy` because unset returns `None` / false for both:
/// callers use this to distinguish "explicit off" from "unset".
pub fn is_env_definitely_falsy(name: &str) -> bool {
    match std::env::var(name) {
        Ok(v) => matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        ),
        Err(_) => false,
    }
}

/// `$CLAUDE_CONFIG_DIR` with `~/.claude` fallback. Matches TS
/// `getClaudeConfigHomeDir`.
pub fn get_claude_config_home_dir() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("CLAUDE_CONFIG_DIR") {
        return std::path::PathBuf::from(dir);
    }
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/"))
        .join(".claude")
}

/// Is --bare / SIMPLE mode on? Controls memory + background services.
pub fn is_bare_mode() -> bool {
    is_env_truthy("CLAUDE_CODE_SIMPLE")
}

// ── Async sleep / timeout ────────────────────────────────────────────────

/// Abort-responsive sleep. Resolves after `duration`, or immediately
/// when `cancel` fires. Matches TS `sleep`.
///
/// Call `cancel.is_cancelled()` after the await if you want to
/// distinguish timer-complete from cancellation.
pub async fn sleep(duration: Duration, cancel: Option<&CancellationToken>) {
    match cancel {
        Some(c) => {
            tokio::select! {
                _ = tokio::time::sleep(duration) => {}
                _ = c.cancelled() => {}
            }
        }
        None => tokio::time::sleep(duration).await,
    }
}

/// Race a future against a timeout. Returns Err when `duration`
/// elapses first. The cancellation doesn't propagate to the inner
/// future — matches TS `withTimeout` (which notes the same limitation).
pub async fn with_timeout<F, T>(duration: Duration, fut: F) -> Result<T, TimeoutError>
where
    F: std::future::Future<Output = T>,
{
    match tokio::time::timeout(duration, fut).await {
        Ok(v) => Ok(v),
        Err(_) => Err(TimeoutError),
    }
}

#[derive(Debug, Error)]
#[error("operation timed out")]
pub struct TimeoutError;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_types_display() {
        let e = ClaudeError("nope".into());
        assert_eq!(e.to_string(), "nope");
        let a = AbortError;
        assert_eq!(a.to_string(), "aborted");
    }

    #[test]
    fn shell_error_includes_code() {
        let se = ShellError {
            stdout: String::new(),
            stderr: String::new(),
            code: 137,
            interrupted: false,
        };
        let s = se.to_string();
        assert!(s.contains("exit code 137"));
    }

    #[test]
    fn config_parse_error_carries_default() {
        let e = ConfigParseError {
            message: "invalid json".into(),
            file_path: "/tmp/x".into(),
            default_config: 42_i32,
        };
        assert_eq!(e.default_config, 42);
        let s = e.to_string();
        assert!(s.contains("/tmp/x"));
        assert!(s.contains("invalid json"));
    }

    #[test]
    fn error_message_uses_display() {
        assert_eq!(error_message(ClaudeError("hi".into())), "hi");
    }

    #[test]
    fn short_error_stack_includes_causes() {
        #[derive(Debug, Error)]
        #[error("outer")]
        struct Outer(#[source] Inner);
        #[derive(Debug, Error)]
        #[error("inner")]
        struct Inner;
        let e = Outer(Inner);
        let s = short_error_stack(&e, 3);
        assert!(s.contains("outer"));
        assert!(s.contains("inner"));
    }

    #[test]
    fn is_enoent_matches_notfound() {
        let e = io::Error::from(io::ErrorKind::NotFound);
        assert!(is_enoent(&e));
        let e2 = io::Error::from(io::ErrorKind::PermissionDenied);
        assert!(!is_enoent(&e2));
    }

    #[test]
    fn is_fs_inaccessible_covers_expected_codes() {
        // Stable ErrorKind variants.
        for kind in [io::ErrorKind::NotFound, io::ErrorKind::PermissionDenied] {
            assert!(is_fs_inaccessible(&io::Error::from(kind)));
        }
        // raw_os_error pathway for ENOTDIR (20) + ELOOP (40 or 62) + EPERM (1).
        for code in [1, 20, 40, 62] {
            assert!(is_fs_inaccessible(&io::Error::from_raw_os_error(code)));
        }
        assert!(!is_fs_inaccessible(&io::Error::from(
            io::ErrorKind::InvalidData
        )));
    }

    #[test]
    fn env_truthy_and_falsy() {
        std::env::set_var("TEST_ENV_TRUTHY_A", "1");
        std::env::set_var("TEST_ENV_TRUTHY_B", "false");
        std::env::remove_var("TEST_ENV_TRUTHY_C");
        assert!(is_env_truthy("TEST_ENV_TRUTHY_A"));
        assert!(!is_env_truthy("TEST_ENV_TRUTHY_B"));
        assert!(!is_env_truthy("TEST_ENV_TRUTHY_C"));
        assert!(is_env_definitely_falsy("TEST_ENV_TRUTHY_B"));
        assert!(!is_env_definitely_falsy("TEST_ENV_TRUTHY_A"));
        assert!(!is_env_definitely_falsy("TEST_ENV_TRUTHY_C"));
        std::env::remove_var("TEST_ENV_TRUTHY_A");
        std::env::remove_var("TEST_ENV_TRUTHY_B");
    }

    #[test]
    fn claude_config_home_dir_respects_override() {
        std::env::set_var("CLAUDE_CONFIG_DIR", "/tmp/myclaude");
        assert_eq!(
            get_claude_config_home_dir(),
            std::path::PathBuf::from("/tmp/myclaude")
        );
        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }

    #[tokio::test(start_paused = true)]
    async fn sleep_resolves_after_duration() {
        let start = tokio::time::Instant::now();
        sleep(Duration::from_millis(100), None).await;
        assert!(start.elapsed() >= Duration::from_millis(100));
    }

    #[tokio::test(start_paused = true)]
    async fn sleep_short_circuits_on_cancel() {
        let cancel = CancellationToken::new();
        let c2 = cancel.clone();
        let handle = tokio::spawn(async move {
            sleep(Duration::from_secs(60), Some(&c2)).await;
        });
        // Cancel immediately — sleep should resolve without waiting 60s.
        cancel.cancel();
        handle.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn with_timeout_returns_ok_when_inner_finishes() {
        let out: Result<&str, _> = with_timeout(Duration::from_secs(5), async { "done" }).await;
        assert_eq!(out.unwrap(), "done");
    }

    #[tokio::test(start_paused = true)]
    async fn with_timeout_errors_when_inner_hangs() {
        let fut = async {
            tokio::time::sleep(Duration::from_secs(60)).await;
            "never"
        };
        let out: Result<&str, _> = with_timeout(Duration::from_millis(10), fut).await;
        assert!(out.is_err());
    }
}
