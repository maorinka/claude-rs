//! `InteractiveToolHost` — production-shaped host that routes
//! tool capabilities through an [`AppStateHandle`].
//!
//! Closes the step-3 architecture wiring the Codex CR called
//! for (final pass, 2026-04-20): tools see only the `ToolHost`
//! trait; the host owns state policy by dispatching typed
//! `AppStateUpdate` variants internally.
//!
//! # What this host is (and isn't)
//!
//! **Is**: a host that propagates state mutations (tool-use
//! tracking, response-char accounting, file-history,
//! attribution, system-message injection) to the actor-backed
//! `AppState` store. Suitable for SDK / agentic sessions where
//! state is shared across parallel tools + consumers subscribe
//! to snapshots.
//!
//! **Isn't**: a REPL host. `request_prompt` and
//! `handle_elicitation` return `None` — this host has no UI
//! surface for interactive dialogs. Integrating with a REPL
//! would wrap this type with an outer host that short-circuits
//! those two methods via a real UI channel, delegating the
//! state methods through.
//!
//! # Production-host invariants
//!
//! The four correctness-critical methods called out in
//! `tool_host` rustdoc are all implemented here — defaults are
//! NOT inherited for transcript / state paths:
//! - `app_state_snapshot` → typed snapshot from the handle.
//! - `mark_tool_use_in_progress` → `AppStateUpdate::MarkToolUseInProgress`.
//! - `update_file_history` → `AppStateUpdate::AppendFileEdit`.
//! - `append_system_message` → `AppStateUpdate::AppendSystemMessage`.
//!
//! The optional ones also route where sensible:
//! - `record_response_chars` → `AppStateUpdate::AddResponseChars`.
//! - `update_attribution` → `AppStateUpdate::AppendAttribution`.
//!
//! # OS notifications
//!
//! `send_os_notification` is a no-op by default. Callers that
//! need iTerm2/Kitty/Ghostty/bell dispatch can wrap this host
//! or call a separate notifier channel — routing OS-level
//! side effects through the actor store would force every
//! subscriber to see + ignore them, which is the wrong
//! primitive (they're not state).

use crate::app_state::{AppStateHandle, AppStateUpdate, AttributionEntry, FileEdit};
use crate::tool_host::{OsNotification, PromptRequest, PromptResponse, ToolHost};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// Production-shaped `ToolHost` wrapping an `AppStateHandle`.
///
/// Clone to hand the same underlying state store to multiple
/// tool invocations — the handle is cheap to clone and routes
/// every update back to the one owner task.
#[derive(Clone)]
pub struct InteractiveToolHost {
    state: AppStateHandle,
}

impl InteractiveToolHost {
    /// Build a host over a fresh actor. Spawns the owner task.
    pub fn spawn() -> Self {
        Self {
            state: AppStateHandle::spawn(),
        }
    }

    /// Build a host over an existing handle. Used when the
    /// caller wants to share the same state across multiple
    /// hosts (e.g. a REPL wrapper layering prompt handling).
    pub fn from_handle(state: AppStateHandle) -> Self {
        Self { state }
    }

    /// Borrow the backing handle. Exposes `snapshot()`,
    /// `subscribe()`, and direct `update()` for application-
    /// layer callers — NOT for tools, which must route through
    /// the `ToolHost` trait per the capability-boundary
    /// contract.
    pub fn handle(&self) -> &AppStateHandle {
        &self.state
    }
}

#[async_trait]
impl ToolHost for InteractiveToolHost {
    async fn app_state_snapshot(&self) -> Option<Arc<crate::app_state::AppState>> {
        Some(self.state.snapshot())
    }

    async fn mark_tool_use_in_progress(&self, tool_use_id: &str, in_progress: bool) {
        let _ = self.state.update(AppStateUpdate::MarkToolUseInProgress {
            tool_use_id: tool_use_id.to_owned(),
            in_progress,
        });
    }

    async fn record_response_chars(&self, chars: usize) {
        let _ = self
            .state
            .update(AppStateUpdate::AddResponseChars { chars });
    }

    async fn update_file_history(&self, edit: FileEdit) {
        let _ = self.state.update(AppStateUpdate::AppendFileEdit(edit));
    }

    async fn update_attribution(&self, entry: AttributionEntry) {
        let _ = self.state.update(AppStateUpdate::AppendAttribution(entry));
    }

    async fn append_system_message(&self, msg: Value) {
        let _ = self.state.update(AppStateUpdate::AppendSystemMessage(msg));
    }

    /// Deliberate no-op: OS-level side effects don't belong in
    /// the actor store (see module docs). Callers needing
    /// notifications wrap this host with a layer that intercepts
    /// `send_os_notification` before delegating.
    async fn send_os_notification(&self, _notif: OsNotification) {}

    /// Returns `None`: no interactive prompt surface. REPL hosts
    /// wrap this with a prompt-aware outer host.
    async fn request_prompt(
        &self,
        _request: PromptRequest,
        _cancel: CancellationToken,
    ) -> Option<PromptResponse> {
        None
    }

    /// Returns `None`: no elicitation surface (same reason as
    /// `request_prompt`).
    async fn handle_elicitation(
        &self,
        _server_name: &str,
        _params: Value,
        _cancel: CancellationToken,
    ) -> Option<Value> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn settle() {
        for _ in 0..3 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn spawn_produces_snapshot_immediately() {
        let host = InteractiveToolHost::spawn();
        let snap = host.app_state_snapshot().await;
        assert!(snap.is_some());
        let s = snap.unwrap();
        assert!(s.in_progress_tool_uses.is_empty());
        assert_eq!(s.response_length, 0);
    }

    #[tokio::test]
    async fn mark_tool_use_routes_through_handle() {
        let host = InteractiveToolHost::spawn();
        host.mark_tool_use_in_progress("tu-1", true).await;
        host.mark_tool_use_in_progress("tu-2", true).await;
        settle().await;
        let snap = host.app_state_snapshot().await.unwrap();
        assert!(snap.in_progress_tool_uses.contains("tu-1"));
        assert!(snap.in_progress_tool_uses.contains("tu-2"));
        host.mark_tool_use_in_progress("tu-1", false).await;
        settle().await;
        let snap = host.app_state_snapshot().await.unwrap();
        assert!(!snap.in_progress_tool_uses.contains("tu-1"));
        assert!(snap.in_progress_tool_uses.contains("tu-2"));
    }

    #[tokio::test]
    async fn response_chars_accumulate() {
        let host = InteractiveToolHost::spawn();
        host.record_response_chars(500).await;
        host.record_response_chars(250).await;
        settle().await;
        assert_eq!(
            host.app_state_snapshot().await.unwrap().response_length,
            750
        );
    }

    #[tokio::test]
    async fn file_history_and_attribution_land_in_state() {
        let host = InteractiveToolHost::spawn();
        host.update_file_history(FileEdit {
            path: "/src/main.rs".into(),
            timestamp_ms: 100,
            kind: "update".into(),
        })
        .await;
        host.update_attribution(AttributionEntry {
            file_path: "/src/main.rs".into(),
            timestamp_ms: 100,
            agent_id: Some("agent-x".into()),
        })
        .await;
        settle().await;
        let snap = host.app_state_snapshot().await.unwrap();
        assert_eq!(snap.file_history.edits.len(), 1);
        assert_eq!(snap.file_history.edits[0].path, "/src/main.rs");
        assert_eq!(snap.attribution.entries.len(), 1);
        assert_eq!(
            snap.attribution.entries[0].agent_id.as_deref(),
            Some("agent-x")
        );
    }

    #[tokio::test]
    async fn append_system_message_routes_through_handle() {
        let host = InteractiveToolHost::spawn();
        host.append_system_message(serde_json::json!({
            "type": "system",
            "subtype": "hook_output",
            "content": "post-tool hook fired",
        }))
        .await;
        settle().await;
        let snap = host.app_state_snapshot().await.unwrap();
        assert_eq!(snap.system_messages.len(), 1);
        assert_eq!(snap.system_messages[0]["subtype"], "hook_output");
    }

    #[tokio::test]
    async fn send_os_notification_is_noop() {
        let host = InteractiveToolHost::spawn();
        host.send_os_notification(OsNotification {
            message: "hello".into(),
            kind: "info".into(),
        })
        .await;
        settle().await;
        // Notifications don't accumulate in state.
        let snap = host.app_state_snapshot().await.unwrap();
        assert!(snap.system_messages.is_empty());
    }

    #[tokio::test]
    async fn prompt_and_elicit_return_none_for_non_interactive() {
        let host = InteractiveToolHost::spawn();
        let cancel = CancellationToken::new();
        let resp = host
            .request_prompt(
                PromptRequest {
                    source: "test".into(),
                    payload: Value::Null,
                },
                cancel.clone(),
            )
            .await;
        assert!(resp.is_none());
        let e = host.handle_elicitation("server", Value::Null, cancel).await;
        assert!(e.is_none());
    }

    #[tokio::test]
    async fn cloning_shares_backing_state() {
        let host1 = InteractiveToolHost::spawn();
        let host2 = host1.clone();
        // Update via host1, read via host2 — they share the actor.
        host1.record_response_chars(1000).await;
        settle().await;
        let snap_from_other = host2.app_state_snapshot().await.unwrap();
        assert_eq!(snap_from_other.response_length, 1000);
    }

    #[tokio::test]
    async fn from_handle_reuses_existing_store() {
        let existing = AppStateHandle::spawn();
        existing
            .update(AppStateUpdate::AddResponseChars { chars: 42 })
            .unwrap();
        settle().await;
        let host = InteractiveToolHost::from_handle(existing);
        let snap = host.app_state_snapshot().await.unwrap();
        assert_eq!(snap.response_length, 42);
    }

    #[tokio::test]
    async fn handle_accessor_exposes_full_subscribe_api() {
        // Application-layer (not tool-layer) callers can
        // subscribe for snapshot streams via the handle.
        let host = InteractiveToolHost::spawn();
        let mut rx = host.handle().subscribe();
        rx.mark_unchanged();
        host.record_response_chars(7).await;
        tokio::time::timeout(std::time::Duration::from_secs(1), rx.changed())
            .await
            .expect("timeout on snapshot stream")
            .unwrap();
        assert_eq!(rx.borrow().response_length, 7);
    }

    #[tokio::test]
    async fn shares_via_arc_dyn_tool_host() {
        // The host satisfies `ToolHost + Send + Sync` so
        // multiple tools can share one `Arc<dyn ToolHost>`.
        let host: Arc<dyn ToolHost> = Arc::new(InteractiveToolHost::spawn());
        host.mark_tool_use_in_progress("tu-arc", true).await;
        settle().await;
        let snap = host.app_state_snapshot().await.unwrap();
        assert!(snap.in_progress_tool_uses.contains("tu-arc"));
    }
}
