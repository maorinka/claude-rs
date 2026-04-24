//! The capability boundary between tools and the host session.
//!
//! **Design decision** (Codex design review, 2026-04-20,
//! gpt-5.4): Rather than faithfully porting TS
//! `ToolUseContext`'s ~40 callback fields into Rust via
//! `Arc<dyn Fn>` fields, we model host capabilities as a **single
//! `ToolHost` trait object** that tools call into. This captures
//! the *legitimate* tool → session surface (permission prompting,
//! state mutation, transcript append, OS notifications) while
//! deliberately **excluding UI-only concerns** that belong in the
//! `claude-tui` / REPL layer.
//!
//! Ports of TS `Tool.ts:158-300` and surrounding hooks.
//!
//! # Kill-list: what this trait does NOT expose
//!
//! The TS `ToolUseContext` has a sprawl of UI / SDK-orchestration
//! fields that are purely presentation or REPL-loop state. These
//! are **deliberately omitted** from `ToolHost`:
//!
//! | TS field                              | Reason omitted                                     |
//! | ------------------------------------- | -------------------------------------------------- |
//! | `setToolJSX`                          | Ink/React render, UI-layer only                    |
//! | `setStreamMode`                       | Spinner animation state, TUI-only                  |
//! | `openMessageSelector`                 | Interactive message-list selector, TUI-only        |
//! | `setSDKStatus`                        | SDK client-facing status, orchestrator concern     |
//! | `onCompactProgress`                   | Compaction UI progress events                      |
//! | `setHasInterruptibleToolInProgress`   | Spinner + escape-key handling, TUI-only            |
//! | `setConversationId`                   | SDK orchestrator, not tool-owned                   |
//! | `setMessages`                         | JSX command callback, UI-only                      |
//! | `abortController`                     | Cancellation reaches tools directly via            |
//! |                                       | `ToolExecutor::call(_, _, cancel, _)` — not host   |
//! | theme / IDE-install / resume hooks    | UI affordances                                     |
//! | footer selection, expanded panels     | View-model, belongs in TUI state                   |
//!
//! Tools that try to reach these via TS historically were leaking
//! UI coupling down into business logic. The Rust port treats this
//! boundary as a one-way permission: tools may ask the host for
//! session-scoped services (permission prompts, state mutations,
//! transcript appends, OS notifications). The TUI renders results
//! the tool returned, plus its own view-model state.
//!
//! # Production-host invariants (step 2 wiring)
//!
//! The default `{}` bodies are ergonomic for tests + batch /
//! non-interactive hosts, but they make it easy to silently
//! no-op correctness-critical host behaviour in a production
//! REPL host. Before step 2 wires tools through the real
//! interactive host, consumers **MUST** implement these
//! (any tool that transits a session turn depends on them):
//!
//! - `app_state_snapshot` — callers reading AppState before a
//!   decision will see `Null` otherwise.
//! - `mark_tool_use_in_progress` — permission UI +
//!   parallel-tool gates depend on this tracker.
//! - `update_file_history` — write validation + diff rendering
//!   depend on the edit log.
//! - `append_system_message` — hook-output surfacing drops
//!   entries silently if defaulted.
//!
//! Truly optional defaults (keep the `{}`):
//! `send_os_notification`, `request_prompt`,
//! `handle_elicitation`, `record_response_chars`,
//! `update_attribution`. Non-interactive / SDK hosts
//! reasonably no-op these.
//!
//! # What IS exposed
//!
//! Every method returns data / a `BoxFuture<…>`. None return JSX
//! or React nodes. Defaults are `Ok(())` / `Ok(None)` so callers
//! can supply a partial implementation (e.g. a `NullToolHost` for
//! batch / non-interactive use).

use crate::app_state::{AppState, AttributionEntry, FileEdit};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// Prompt request a tool can send to the host to ask the user a
/// question mid-execution (e.g. MCP elicitation, permission
/// clarifications).
///
/// Opaque payload — the real shape depends on the elicitation
/// schema and is managed by the caller. Kept as `Value` so this
/// trait stays framework-agnostic.
#[derive(Debug, Clone)]
pub struct PromptRequest {
    pub source: String,
    pub payload: Value,
}

#[derive(Debug, Clone)]
pub struct PromptResponse {
    pub accepted: bool,
    pub payload: Value,
}

/// OS-level notification the tool wants delivered to the user.
#[derive(Debug, Clone)]
pub struct OsNotification {
    pub message: String,
    pub kind: String,
}

/// Session-scoped capabilities exposed to tool implementations.
///
/// All methods are `&self`-borrowed so the trait is object-safe
/// and multiple tools can share one `Arc<dyn ToolHost>`.
/// State-mutation methods take `Value` as the opaque state shape
/// — step 2 of the port lands an `AppState` actor store that the
/// host wires through; until then, hosts can no-op or pass
/// through a locked shared map.
#[async_trait]
pub trait ToolHost: Send + Sync {
    /// Read-only snapshot of host state. Returns a fresh
    /// `Arc<AppState>` clone; production hosts wire this to an
    /// `AppStateHandle::snapshot()` call. A `None` return
    /// documents "this host doesn't participate in session
    /// state" (batch / SDK / test hosts), distinguishing that
    /// from "state is present but empty". Tools that treat
    /// `None` as an error should fail their invocation rather
    /// than assume defaults.
    async fn app_state_snapshot(&self) -> Option<Arc<AppState>> {
        None
    }

    /// Record that a tool invocation is or isn't in progress.
    /// Matches TS `setInProgressToolUseIDs(prev => ...)`.
    /// Production hosts dispatch an
    /// `AppStateUpdate::MarkToolUseInProgress` internally.
    async fn mark_tool_use_in_progress(&self, _tool_use_id: &str, _in_progress: bool) {}

    /// Record that the tool contributed `chars` to the streamed
    /// response. Hosts that implement context-budget tracking
    /// (REPL compaction) dispatch
    /// `AppStateUpdate::AddResponseChars` internally;
    /// non-interactive hosts no-op.
    async fn record_response_chars(&self, _chars: usize) {}

    /// Append an entry to the host's file-history state.
    /// Production hosts dispatch
    /// `AppStateUpdate::AppendFileEdit(edit)` internally.
    async fn update_file_history(&self, _edit: FileEdit) {}

    /// Append an entry to the host's per-agent attribution
    /// tracker (who edited what). Production hosts dispatch
    /// `AppStateUpdate::AppendAttribution(entry)` internally.
    async fn update_attribution(&self, _entry: AttributionEntry) {}

    /// Inject a system-level message into the transcript. Used by
    /// hook outputs that need to surface text without the model's
    /// next turn producing it. `msg` is the wire-format message
    /// object (see `messages_fold` module for reconstruction).
    async fn append_system_message(&self, _msg: Value) {}

    /// Deliver an OS-level notification (iTerm2, Kitty, Ghostty,
    /// bell, etc.). Non-TTY hosts no-op.
    async fn send_os_notification(&self, _notif: OsNotification) {}

    /// Ask the user a question mid-tool. Returns `None` when the
    /// host can't prompt (non-interactive / batch / SDK-from-
    /// stdin). Interactive REPL hosts return `Some(response)`.
    async fn request_prompt(
        &self,
        _request: PromptRequest,
        _cancel: CancellationToken,
    ) -> Option<PromptResponse> {
        None
    }

    /// Handle a URL-based elicitation from an MCP server.
    /// Returns `None` when the host can't render the dialog.
    async fn handle_elicitation(
        &self,
        _server_name: &str,
        _params: Value,
        _cancel: CancellationToken,
    ) -> Option<Value> {
        None
    }
}

/// No-op implementation for tests + batch / non-interactive
/// callers. Every method returns the trait's default.
#[derive(Debug, Default, Clone)]
pub struct NullToolHost;

#[async_trait]
impl ToolHost for NullToolHost {}

/// Convenience alias for sharing a host across tools.
pub type SharedToolHost = Arc<dyn ToolHost>;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn null_host_returns_trait_defaults() {
        let host: SharedToolHost = Arc::new(NullToolHost);
        assert!(host.app_state_snapshot().await.is_none());
        host.mark_tool_use_in_progress("t1", true).await;
        host.record_response_chars(100).await;
        host.update_file_history(FileEdit {
            path: "/a".into(),
            timestamp_ms: 0,
            kind: "update".into(),
        })
        .await;
        host.update_attribution(AttributionEntry {
            file_path: "/a".into(),
            timestamp_ms: 0,
            agent_id: None,
        })
        .await;
        host.append_system_message(Value::Null).await;
        host.send_os_notification(OsNotification {
            message: "done".into(),
            kind: "info".into(),
        })
        .await;

        let r = host
            .request_prompt(
                PromptRequest {
                    source: "test".into(),
                    payload: Value::Null,
                },
                CancellationToken::new(),
            )
            .await;
        assert!(r.is_none());

        let e = host
            .handle_elicitation("server", Value::Null, CancellationToken::new())
            .await;
        assert!(e.is_none());
    }

    /// Prove a custom host can override individual methods without
    /// implementing the full surface. This is the key ergonomics
    /// claim for the default-method-laden trait.
    #[tokio::test]
    async fn custom_host_partial_override() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct CountingHost {
            prompts_recorded: AtomicUsize,
            chars_recorded: AtomicUsize,
        }

        #[async_trait]
        impl ToolHost for CountingHost {
            async fn mark_tool_use_in_progress(&self, _: &str, _: bool) {
                self.prompts_recorded.fetch_add(1, Ordering::SeqCst);
            }
            async fn record_response_chars(&self, chars: usize) {
                self.chars_recorded.fetch_add(chars, Ordering::SeqCst);
            }
        }

        let host = CountingHost {
            prompts_recorded: AtomicUsize::new(0),
            chars_recorded: AtomicUsize::new(0),
        };
        host.mark_tool_use_in_progress("t1", true).await;
        host.mark_tool_use_in_progress("t2", true).await;
        host.record_response_chars(500).await;
        host.record_response_chars(750).await;
        // Overridden methods counted.
        assert_eq!(host.prompts_recorded.load(Ordering::SeqCst), 2);
        assert_eq!(host.chars_recorded.load(Ordering::SeqCst), 1250);
        // Non-overridden defaults still callable, don't panic.
        host.update_file_history(FileEdit {
            path: "/x".into(),
            timestamp_ms: 0,
            kind: "update".into(),
        })
        .await;
        host.send_os_notification(OsNotification {
            message: "x".into(),
            kind: "y".into(),
        })
        .await;
    }

    /// End-to-end: a production-shaped host that wraps
    /// `AppStateHandle` + forwards host methods to typed
    /// `AppStateUpdate` dispatches. Codex CR step-2 verdict:
    /// tools must not see the handle directly; the host
    /// owns state policy.
    #[tokio::test]
    async fn app_state_backed_host_routes_updates_via_handle() {
        use crate::app_state::AppStateHandle;

        struct BackedHost {
            state: AppStateHandle,
        }

        #[async_trait]
        impl ToolHost for BackedHost {
            async fn app_state_snapshot(&self) -> Option<Arc<AppState>> {
                Some(self.state.snapshot())
            }

            async fn mark_tool_use_in_progress(&self, id: &str, in_progress: bool) {
                let _ =
                    self.state
                        .update(crate::app_state::AppStateUpdate::MarkToolUseInProgress {
                            tool_use_id: id.into(),
                            in_progress,
                        });
            }

            async fn record_response_chars(&self, chars: usize) {
                let _ = self
                    .state
                    .update(crate::app_state::AppStateUpdate::AddResponseChars { chars });
            }

            async fn update_file_history(&self, edit: FileEdit) {
                let _ = self
                    .state
                    .update(crate::app_state::AppStateUpdate::AppendFileEdit(edit));
            }

            async fn update_attribution(&self, entry: AttributionEntry) {
                let _ = self
                    .state
                    .update(crate::app_state::AppStateUpdate::AppendAttribution(entry));
            }
        }

        let host = BackedHost {
            state: AppStateHandle::spawn(),
        };
        // Tools invoke the host, which internally dispatches updates.
        host.mark_tool_use_in_progress("tu-x", true).await;
        host.record_response_chars(500).await;
        host.update_file_history(FileEdit {
            path: "/src/main.rs".into(),
            timestamp_ms: 12345,
            kind: "update".into(),
        })
        .await;
        host.update_attribution(AttributionEntry {
            file_path: "/src/main.rs".into(),
            timestamp_ms: 12345,
            agent_id: Some("a-deadbeef".into()),
        })
        .await;

        // Allow owner loop to drain + publish.
        for _ in 0..3 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        // Snapshot reads typed state, not Value.
        let snap = host.app_state_snapshot().await.expect("snapshot");
        assert!(snap.in_progress_tool_uses.contains("tu-x"));
        assert_eq!(snap.response_length, 500);
        assert_eq!(snap.file_history.edits.len(), 1);
        assert_eq!(snap.file_history.edits[0].path, "/src/main.rs");
        assert_eq!(snap.attribution.entries.len(), 1);
    }

    #[tokio::test]
    async fn trait_object_dispatch_works() {
        // Confirm the trait is object-safe (all methods take `&self`).
        let boxed: Box<dyn ToolHost> = Box::new(NullToolHost);
        boxed.record_response_chars(1).await;
        let _arc: SharedToolHost = Arc::new(NullToolHost);
    }

    #[test]
    fn prompt_request_and_response_cloneable() {
        // Sanity: both carry `Value` + `String`, must clone + debug.
        let req = PromptRequest {
            source: "s".into(),
            payload: Value::from(42),
        };
        let _ = req.clone();
        let resp = PromptResponse {
            accepted: true,
            payload: Value::Null,
        };
        let _ = resp.clone();
    }
}
