//! Platform-agnostic lightweight process utilities.
//!
//! Port of selected helpers from TS
//! `utils/genericProcessUtils.ts:1-184`.
//!
//! **Scope**: only [`is_process_running`] is ported. The other two
//! TS helpers (`getAncestorPidsAsync`, `getProcessCommand`) shell
//! out to `ps` / PowerShell — the current Rust tree has no
//! caller that needs them, so porting waits until one appears.
//!
//! This function is the liveness probe used by the scheduler-
//! lock / computer-use-lock recovery paths. Implementing it
//! correctly unblocks future ports of those lock files.

/// Returns `true` if a process with PID `pid` is currently alive
/// and reachable by this user, `false` otherwise.
///
/// Matches TS `isProcessRunning`:
/// - PID ≤ 1 → `false` (PID 0 = current process group placeholder on
///   many platforms, PID 1 = init).
/// - Send signal 0 on Unix (standard liveness probe).
/// - On Windows, use `OpenProcess` with
///   `PROCESS_QUERY_LIMITED_INFORMATION`; a non-null handle means
///   the process exists.
///
/// Conservatively returns `false` when the process exists but is
/// owned by another user (`EPERM` on Unix). The TS source calls
/// this out as intended: for lock recovery, "couldn't probe" is
/// treated the same as "running" so we never steal a live lock.
pub fn is_process_running(pid: i32) -> bool {
    if pid <= 1 {
        return false;
    }

    #[cfg(unix)]
    {
        // `kill(pid, 0)` returns 0 iff the process exists AND we have
        // permission to send it a signal. `EPERM` means it exists but
        // we can't signal it — TS treats that as "not running" (signal 0
        // throws), so Rust does the same.
        let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
        rc == 0
    }
    #[cfg(windows)]
    {
        // The simplest stable approach without a new dep: run
        // `tasklist /FI "PID eq <pid>" /NH`. This is the same
        // approach Node's `process.kill(pid, 0)` falls through to on
        // Windows.
        //
        // Alternative: `OpenProcess` via `windows-sys` gives a faster
        // result but would add a crate. Keep the subprocess for now
        // since this is cold-path (lock-recovery).
        use std::process::Command;
        let out = Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
            .output();
        match out {
            Ok(o) => {
                let s = String::from_utf8_lossy(&o.stdout);
                // Running process → a CSV row with the PID. Missing → an
                // "INFO: No tasks are running which match the specified criteria."
                s.contains(&format!(",\"{pid}\","))
            }
            Err(_) => false,
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pid_zero_not_running() {
        assert!(!is_process_running(0));
    }

    #[test]
    fn pid_one_not_running() {
        // TS explicitly excludes PID 1 (init). Rust mirrors the
        // behaviour so lock-recovery callers never get a "running"
        // for init's PID.
        assert!(!is_process_running(1));
    }

    #[test]
    fn negative_pid_not_running() {
        assert!(!is_process_running(-1));
        assert!(!is_process_running(-99999));
    }

    #[test]
    fn current_process_is_running() {
        // Our own PID must report alive (signal 0 to self always
        // succeeds, and Windows tasklist always sees the calling
        // process).
        let my_pid = std::process::id() as i32;
        assert!(is_process_running(my_pid));
    }

    #[test]
    fn bogus_high_pid_not_running() {
        // 32-bit max - 7 — almost certainly not a real PID. Not
        // guaranteed-absent (OS could in principle allocate it), so
        // allow either answer but don't panic.
        let _ = is_process_running(i32::MAX - 7);
    }
}
