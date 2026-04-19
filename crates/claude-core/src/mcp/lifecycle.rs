//! Connection lifecycle error tracking for MCP transports.
//!
//! Gap-fill ticket **G4a** in the MCP client plan. Ports the
//! classifier + counter + reconnect-trigger logic from
//! `src/services/mcp/client.ts:1221-1391`:
//!
//! - `is_terminal_connection_error(msg)` — the substring-pattern
//!   match TS uses at `client.ts:1249-1263` to decide whether an
//!   error message signals a terminated connection we should try
//!   to reconnect, or a transient issue we can absorb.
//! - `LifecycleTracker` — the state machine around
//!   `consecutiveConnectionErrors` + `MAX_ERRORS_BEFORE_RECONNECT`
//!   + `hasTriggeredClose` from `client.ts:1227-1247,1331-1365`.
//!
//! **Scope**: this module is pure data/logic — classifier,
//! counter, and reconnect-trigger decision. Wiring the tracker
//! into the stdio / SSE / HTTP reader error paths is a follow-up
//! ticket (G4b). Keeping them split so the classifier can be unit
//! tested exhaustively without transport mocks.

/// TS `MAX_ERRORS_BEFORE_RECONNECT` at `client.ts:1228`. After this
/// many consecutive terminal connection errors, the tracker signals
/// the transport should close and the caller should reconnect.
pub const MAX_ERRORS_BEFORE_RECONNECT: u32 = 3;

/// Every substring the TS classifier at `client.ts:1249-1263`
/// treats as "terminal" — i.e. the connection is broken badly
/// enough that retry-in-place is hopeless and a full reconnect is
/// required.
///
/// Verbatim from TS. The `SSE stream disconnected` / `Failed to
/// reconnect SSE stream` entries catch wrapper messages the MCP
/// SDK prepends around the actual network errno, which a bare
/// `ECONNRESET` substring search would miss.
const TERMINAL_ERROR_SUBSTRINGS: &[&str] = &[
    "ECONNRESET",
    "ETIMEDOUT",
    "EPIPE",
    "EHOSTUNREACH",
    "ECONNREFUSED",
    "Body Timeout Error",
    "terminated",
    "SSE stream disconnected",
    "Failed to reconnect SSE stream",
];

/// Classify an error message as a terminal connection failure.
/// Matches TS `isTerminalConnectionError` at `client.ts:1249-1263`.
/// Case-sensitive substring match — TS uses `String.includes`,
/// which is also case-sensitive.
pub fn is_terminal_connection_error(msg: &str) -> bool {
    TERMINAL_ERROR_SUBSTRINGS
        .iter()
        .any(|needle| msg.contains(needle))
}

/// What the tracker wants the caller to do after a classified
/// error. Ports the effects of TS's inline branching at
/// `client.ts:1333-1364`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifecycleDecision {
    /// No action required — either the error was non-terminal, or
    /// the terminal counter hasn't reached the threshold yet.
    Continue,
    /// Close the transport and reject pending requests. TS calls
    /// `closeTransportAndRejectPending(reason)`; the caller's
    /// transport layer is responsible for the actual teardown. The
    /// carried string is the same reason string TS logs, so logs
    /// stay aligned across the port.
    TriggerClose { reason: &'static str },
}

/// Stateful tracker for one connection's lifecycle error signal.
///
/// Call `record_error(msg)` once per `onerror`-equivalent event;
/// the returned `LifecycleDecision` tells the caller whether to
/// keep going or tear the transport down. `record_session_expired`
/// is a dedicated path for the HTTP 404 + JSON-RPC -32001 signal
/// — it always returns `TriggerClose` on the first call and
/// `Continue` thereafter (idempotent).
///
/// Non-terminal errors reset the counter, matching TS's "transient
/// issue absorbed" semantics at `client.ts:1361-1364`.
///
/// `Default` produces a clean tracker (0 errors, not yet closed).
/// Thread-safe by construction? No — wrap in `Mutex` if shared
/// across tasks. Keeping it unsynchronised internally mirrors TS's
/// closure-captured `let` variables and lets callers choose their
/// own locking strategy.
///
/// # Transport gating (G4b wiring note)
/// TS scopes the terminal-error counter and the
/// `"Maximum reconnection attempts"` short-circuit to remote
/// transports only — specifically `sse`, `http`, and
/// `claudeai-proxy` (see `client.ts:1333-1364`). The tracker here
/// is intentionally transport-agnostic so the data structure
/// stays composable, but callers wiring it into transport reader
/// loops (G4b) should NOT apply `record_error` to stdio reader
/// errors: a crashed stdio subprocess surfaces as a process-exit
/// signal, not as a reconnectable network-level flap, and running
/// it through the counter would diverge from TS behaviour.
#[derive(Debug, Default, Clone)]
pub struct LifecycleTracker {
    consecutive_errors: u32,
    has_triggered_close: bool,
}

impl LifecycleTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot of the internal counter — mostly for telemetry /
    /// debug logging. `0` after a successful exchange or a
    /// non-terminal error.
    pub fn consecutive_errors(&self) -> u32 {
        self.consecutive_errors
    }

    /// `true` once a `TriggerClose` has been returned. Matches TS
    /// `hasTriggeredClose` guard at `client.ts:1232,1241`.
    pub fn has_triggered_close(&self) -> bool {
        self.has_triggered_close
    }

    /// Record an `onerror`-equivalent event. The message is
    /// classified; terminal errors bump the counter and may trigger
    /// a close when the threshold is reached.
    ///
    /// The special "Maximum reconnection attempts" SDK wrapper from
    /// TS `client.ts:1342-1348` is handled here too: it
    /// unconditionally triggers close with reason `"SSE
    /// reconnection exhausted"`, bypassing the counter.
    pub fn record_error(&mut self, msg: &str) -> LifecycleDecision {
        if self.has_triggered_close {
            return LifecycleDecision::Continue;
        }

        // TS `client.ts:1342`: SDK's "gave up reconnecting" wrapper
        // — definitive teardown signal, no counter check.
        if msg.contains("Maximum reconnection attempts") {
            self.has_triggered_close = true;
            return LifecycleDecision::TriggerClose {
                reason: "SSE reconnection exhausted",
            };
        }

        if is_terminal_connection_error(msg) {
            self.consecutive_errors += 1;
            if self.consecutive_errors >= MAX_ERRORS_BEFORE_RECONNECT {
                self.consecutive_errors = 0;
                self.has_triggered_close = true;
                return LifecycleDecision::TriggerClose {
                    reason: "max consecutive terminal errors",
                };
            }
            return LifecycleDecision::Continue;
        }

        // Transient / non-terminal → absorb and reset the streak.
        // TS `client.ts:1361-1364`.
        self.consecutive_errors = 0;
        LifecycleDecision::Continue
    }

    /// Record a session-expired event (HTTP 404 + JSON-RPC -32001)
    /// — TS `client.ts:1316-1329`. Always closes on first call,
    /// idempotent thereafter.
    pub fn record_session_expired(&mut self) -> LifecycleDecision {
        if self.has_triggered_close {
            return LifecycleDecision::Continue;
        }
        self.has_triggered_close = true;
        LifecycleDecision::TriggerClose {
            reason: "session expired",
        }
    }

    /// Mark a clean disconnect so subsequent `record_error` calls
    /// become no-ops. Callers should invoke this from their
    /// `onclose` equivalent path.
    pub fn mark_closed(&mut self) {
        self.has_triggered_close = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── classifier ──────────────────────────────────────────────

    #[test]
    fn classifier_matches_each_terminal_substring() {
        for needle in TERMINAL_ERROR_SUBSTRINGS {
            let msg = format!("something went wrong: {}", needle);
            assert!(
                is_terminal_connection_error(&msg),
                "substring {:?} should classify as terminal",
                needle
            );
        }
    }

    #[test]
    fn classifier_rejects_non_terminal_errors() {
        assert!(!is_terminal_connection_error("401 Unauthorized"));
        assert!(!is_terminal_connection_error("parse error near line 3"));
        assert!(!is_terminal_connection_error(""));
        // TS classifier is case-sensitive: lowercase econnreset
        // doesn't match. Preserve parity — don't introduce an
        // accidental case-insensitive relaxation.
        assert!(!is_terminal_connection_error("econnreset"));
    }

    #[test]
    fn classifier_matches_wrapped_sdk_messages() {
        assert!(is_terminal_connection_error(
            "Failed to reconnect SSE stream after 3 attempts: network down"
        ));
        assert!(is_terminal_connection_error(
            "SSE stream disconnected while reading"
        ));
    }

    // ─── tracker ─────────────────────────────────────────────────

    #[test]
    fn tracker_starts_clean() {
        let t = LifecycleTracker::new();
        assert_eq!(t.consecutive_errors(), 0);
        assert!(!t.has_triggered_close());
    }

    #[test]
    fn tracker_counts_terminal_errors_up_to_threshold() {
        let mut t = LifecycleTracker::new();
        assert_eq!(
            t.record_error("ECONNRESET happened"),
            LifecycleDecision::Continue
        );
        assert_eq!(t.consecutive_errors(), 1);
        assert_eq!(
            t.record_error("ETIMEDOUT happened"),
            LifecycleDecision::Continue
        );
        assert_eq!(t.consecutive_errors(), 2);
        // Third → close fires. Counter is reset per TS line 1358.
        assert_eq!(
            t.record_error("EPIPE happened"),
            LifecycleDecision::TriggerClose {
                reason: "max consecutive terminal errors"
            }
        );
        assert!(t.has_triggered_close());
        assert_eq!(t.consecutive_errors(), 0);
    }

    #[test]
    fn tracker_non_terminal_error_resets_counter() {
        let mut t = LifecycleTracker::new();
        t.record_error("ECONNRESET");
        t.record_error("ETIMEDOUT");
        assert_eq!(t.consecutive_errors(), 2);
        // Non-terminal absorbs and resets the streak.
        let d = t.record_error("401 Unauthorized");
        assert_eq!(d, LifecycleDecision::Continue);
        assert_eq!(t.consecutive_errors(), 0);
    }

    #[test]
    fn tracker_max_reconnection_bypasses_counter() {
        let mut t = LifecycleTracker::new();
        // First-ever error, but SDK wrapper is definitive.
        let d = t.record_error("Maximum reconnection attempts exceeded");
        assert_eq!(
            d,
            LifecycleDecision::TriggerClose {
                reason: "SSE reconnection exhausted"
            }
        );
        assert!(t.has_triggered_close());
    }

    #[test]
    fn tracker_close_is_idempotent() {
        let mut t = LifecycleTracker::new();
        t.record_error("Maximum reconnection attempts");
        // Subsequent calls are no-ops.
        assert_eq!(t.record_error("ECONNRESET"), LifecycleDecision::Continue);
        assert_eq!(t.record_session_expired(), LifecycleDecision::Continue);
    }

    #[test]
    fn tracker_session_expired_closes_once() {
        let mut t = LifecycleTracker::new();
        assert_eq!(
            t.record_session_expired(),
            LifecycleDecision::TriggerClose {
                reason: "session expired"
            }
        );
        // Second call is absorbed.
        assert_eq!(t.record_session_expired(), LifecycleDecision::Continue);
    }

    #[test]
    fn tracker_non_terminal_reset_requires_fresh_three_to_fire_close() {
        // Codex CR coverage gap: a non-terminal mid-streak must
        // fully reset — three FRESH terminal errors after the
        // reset should be needed to trigger close. Guards against
        // an off-by-one where we might decrement-to-2 instead of
        // zeroing.
        let mut t = LifecycleTracker::new();
        t.record_error("ECONNRESET"); // streak=1
        t.record_error("ETIMEDOUT"); // streak=2
        t.record_error("401 Unauthorized"); // non-terminal → reset
        assert_eq!(t.consecutive_errors(), 0);
        // Two fresh terminal must NOT fire close.
        assert_eq!(
            t.record_error("ECONNRESET"),
            LifecycleDecision::Continue
        );
        assert_eq!(
            t.record_error("ECONNRESET"),
            LifecycleDecision::Continue
        );
        // Third fires.
        assert_eq!(
            t.record_error("ECONNRESET"),
            LifecycleDecision::TriggerClose {
                reason: "max consecutive terminal errors"
            }
        );
    }

    #[test]
    fn tracker_mark_closed_suppresses_subsequent_signals() {
        let mut t = LifecycleTracker::new();
        t.mark_closed();
        assert_eq!(t.record_error("ECONNRESET"), LifecycleDecision::Continue);
        assert_eq!(t.record_error("ECONNRESET"), LifecycleDecision::Continue);
        assert_eq!(t.record_session_expired(), LifecycleDecision::Continue);
    }
}
