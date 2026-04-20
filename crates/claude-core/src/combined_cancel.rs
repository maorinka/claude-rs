//! Combine cancellation tokens + optional timeout into one child token.
//!
//! Port of TS `utils/combinedAbortSignal.ts:1-47`.
//!
//! TS uses `AbortSignal` / `AbortController`; Rust uses
//! `tokio_util::sync::CancellationToken`. The shape is the same: a
//! new child token that flips when any of the inputs flip OR the
//! timeout elapses.
//!
//! Why this module exists
//! ======================
//! TS chose not to pass `AbortSignal.timeout(ms)` as one of the input
//! signals because under Bun those timers were GC'd lazily and
//! accumulated native memory. Rust doesn't share that quirk, but
//! having `combined_cancel(parent, Some(sibling), Some(timeout))`
//! reads cleaner than manually chaining `select!` branches, so the
//! helper stays.

use std::time::Duration;
use tokio_util::sync::{CancellationToken, DropGuard};

/// Result of combining cancellation sources: the new child token and
/// a guard that — when dropped — aborts the background watcher task
/// started for signal B / timeout.
pub struct Combined {
    pub token: CancellationToken,
    /// Drop this to release the watcher. If the combined token has
    /// already fired, the watcher has already exited; dropping is a
    /// no-op in that case.
    #[allow(dead_code)]
    _guard: Option<DropGuard>,
}

/// Build a combined cancellation.
///
/// - If `primary` or `secondary` is already cancelled, the returned
///   token is cancelled immediately.
/// - Otherwise a child token is created as a child of `primary`
///   (TS sets up listeners; Rust uses `CancellationToken::child_token`
///   to chain).
/// - `secondary` and `timeout` are wired up via a spawned watcher task.
///
/// The returned [`Combined`] holds a guard that ensures the watcher
/// stops when the caller drops the result, matching TS's explicit
/// `cleanup()` contract.
pub fn combined_cancel(
    primary: Option<&CancellationToken>,
    secondary: Option<&CancellationToken>,
    timeout: Option<Duration>,
) -> Combined {
    let primary_aborted = primary.is_some_and(|t| t.is_cancelled());
    let secondary_aborted = secondary.is_some_and(|t| t.is_cancelled());

    if primary_aborted || secondary_aborted {
        // Short-circuit: already cancelled. No watcher needed.
        let token = CancellationToken::new();
        token.cancel();
        return Combined {
            token,
            _guard: None,
        };
    }

    // Create the child as a child of `primary` when available, so
    // cancelling `primary` propagates via the tokio_util chain
    // (cheaper than a dedicated watcher). Fall back to a free-standing
    // token when there's no primary.
    let child = match primary {
        Some(p) => p.child_token(),
        None => CancellationToken::new(),
    };

    // Watch for `secondary` / timeout only when at least one is
    // supplied. The watcher cancels `child` on the first trigger, then
    // exits.
    if secondary.is_some() || timeout.is_some() {
        let child_for_task = child.clone();
        let secondary_clone = secondary.cloned();
        tokio::spawn(async move {
            tokio::select! {
                _ = async {
                    if let Some(s) = secondary_clone.as_ref() {
                        s.cancelled().await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                } => {
                    child_for_task.cancel();
                }
                _ = async {
                    if let Some(d) = timeout {
                        tokio::time::sleep(d).await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                } => {
                    child_for_task.cancel();
                }
                // Caller dropped the Combined → child was cancelled via
                // the guard, so this arm fires and we exit quietly.
                _ = child_for_task.cancelled() => {}
            }
        });
    }

    // Returning a DropGuard means: when caller drops `Combined`,
    // `child` is cancelled, which in turn ends the watcher. Matches
    // TS's `cleanup()` semantics without making the caller call it.
    let guard = child.clone().drop_guard();
    Combined {
        token: child,
        _guard: Some(guard),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{sleep, Duration};

    #[tokio::test]
    async fn primary_already_cancelled_short_circuits() {
        let p = CancellationToken::new();
        p.cancel();
        let c = combined_cancel(Some(&p), None, None);
        assert!(c.token.is_cancelled());
    }

    #[tokio::test]
    async fn secondary_already_cancelled_short_circuits() {
        let s = CancellationToken::new();
        s.cancel();
        let c = combined_cancel(None, Some(&s), None);
        assert!(c.token.is_cancelled());
    }

    #[tokio::test]
    async fn primary_cancellation_propagates() {
        let p = CancellationToken::new();
        let c = combined_cancel(Some(&p), None, None);
        assert!(!c.token.is_cancelled());
        p.cancel();
        // Child token propagation from `tokio_util` is synchronous.
        assert!(c.token.is_cancelled());
    }

    #[tokio::test]
    async fn secondary_cancellation_propagates() {
        let s = CancellationToken::new();
        let c = combined_cancel(None, Some(&s), None);
        assert!(!c.token.is_cancelled());
        s.cancel();
        // Watcher task needs a yield to observe.
        tokio::task::yield_now().await;
        // Poll briefly for the async path.
        for _ in 0..10 {
            if c.token.is_cancelled() {
                break;
            }
            sleep(Duration::from_millis(5)).await;
        }
        assert!(c.token.is_cancelled());
    }

    #[tokio::test]
    async fn timeout_fires() {
        let c = combined_cancel(None, None, Some(Duration::from_millis(30)));
        assert!(!c.token.is_cancelled());
        sleep(Duration::from_millis(100)).await;
        assert!(c.token.is_cancelled());
    }

    #[tokio::test]
    async fn drop_cancels_child_and_releases_watcher() {
        let s = CancellationToken::new();
        let child_clone = {
            let c = combined_cancel(None, Some(&s), Some(Duration::from_secs(60)));
            c.token.clone()
        }; // `c` dropped here — drop_guard should cancel.
        assert!(child_clone.is_cancelled(), "drop guard must cancel the child");
    }

    #[tokio::test]
    async fn no_inputs_produces_never_cancelled_token() {
        let c = combined_cancel(None, None, None);
        // Not cancelled and no watcher spawned.
        assert!(!c.token.is_cancelled());
        // Drop it and observe the guard fires cancel.
        let tok = c.token.clone();
        drop(c);
        assert!(tok.is_cancelled());
    }
}
