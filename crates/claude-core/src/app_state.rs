//! `AppState` actor store with typed update messages and snapshot
//! broadcast.
//!
//! **Step 2** of the 3-step design rollout (Codex CLI gpt-5.4,
//! 2026-04-20). Step 1 (`tool_host` capability boundary) kept
//! state fields as `serde_json::Value` placeholders; this step
//! swaps them to typed structures.
//!
//! Design decision: **single-writer actor** — one owner task
//! holds the authoritative `AppState`, consumers send typed
//! `AppStateUpdate` variants over an `mpsc::UnboundedSender`,
//! and snapshots are published via `watch::Sender<Arc<AppState>>`.
//! Codex debate answer on Q1 verbatim:
//! > "The TS shape is already semantically a single-writer
//! > immutable store with reducer-style updates, and Rust should
//! > preserve that concurrency model instead of pretending it is
//! > a normal shared mutable struct."
//!
//! # Scope — what IS in this MVP
//!
//! Fields the `ToolHost` contract (step 1) needs to surface
//! correctness-critical state in a typed way:
//! - `tasks` — map of in-flight task state (still `Value`; TS
//!   `TaskState` is itself a discriminated union with per-variant
//!   fields — typing it is a separate port).
//! - `file_history` — typed edit log consumed by Write/Edit tools
//!   for staleness detection.
//! - `attribution` — typed entries tracking which agent authored
//!   which file change.
//! - `in_progress_tool_uses` — parallel-tool gate set.
//! - `response_length` — streamed-char counter for compaction
//!   triggers.
//! - `agent_name_registry` — name → agent id map set by the
//!   Agent tool (TS `AppStateStore.ts:163`).
//! - `tool_permission_context` — typed
//!   `permissions::types::ToolPermissionContext` (the
//!   canonical type the live permission subsystem uses).
//!
//! # Scope — what is NOT in this MVP
//!
//! Fields with no legitimate tool-access today stay OUT until a
//! real caller surfaces (matches Q7's kill-list discipline):
//! footer selection, expanded panels, companion sprite state,
//! remote session URL, IDE status, pending compaction UI, etc.
//!
//! # Update semantics
//!
//! Every state mutation flows through a typed `AppStateUpdate`
//! variant. New callers that need a mutation pattern not covered
//! by the existing variants **add a variant** — the discipline
//! is intentional: typed messages keep audit trails readable
//! (`Debug` formatting names the variant), serialise cleanly
//! for log replay, and prevent the actor's private state
//! mutation from leaking into the caller closure.
//!
//! Codex CR follow-up (2026-04-20, final pass): an earlier
//! `Reducer(Box<dyn FnOnce>)` escape hatch was removed to enforce
//! this discipline before the state surface grows further.
//!
//! # Shutdown
//!
//! Dropping the last `AppStateHandle` closes the mpsc sender;
//! the owner task observes the channel close and exits. Tests
//! use `spawn_for_tests` which returns a handle + the owner
//! task's `JoinHandle` so the test can await clean shutdown.
//!
//! A retained `watch::Receiver` survives owner-task exit: it
//! continues to expose the last published snapshot via
//! `borrow()`, and `changed().await` returns `Err` once the
//! channel is fully closed.
//!
//! # Coalescing semantics
//!
//! The owner loop drains every queued `AppStateUpdate` via
//! `try_recv` between `recv`s, applying them all BEFORE
//! publishing a single snapshot. This keeps subscriber churn
//! low when tools fire 2-3 updates back-to-back. Subscribers
//! MUST NOT rely on intermediate states being observable — a
//! burst of N updates publishes exactly one snapshot, namely
//! the post-burst state. Callers that need per-update
//! observability want a separate event stream (e.g.
//! `broadcast::Sender<AppStateUpdate>`), not `watch`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::{mpsc, watch};

/// Typed append-only edit log entry. TS `FileHistoryState` has a
/// richer shape; this port captures the fields tools mutate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEdit {
    pub path: String,
    /// Unix-epoch milliseconds.
    pub timestamp_ms: i64,
    /// One of `"create"`, `"update"`, `"delete"`. Not an enum in
    /// TS — kept as string for now to avoid a premature closed-set
    /// decision; callers may widen later (e.g. `"patch"`,
    /// `"rename"`).
    pub kind: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileHistoryState {
    pub edits: Vec<FileEdit>,
}

impl FileHistoryState {
    pub fn push(&mut self, edit: FileEdit) {
        self.edits.push(edit);
    }

    pub fn last_for_path(&self, path: &str) -> Option<&FileEdit> {
        self.edits.iter().rev().find(|e| e.path == path)
    }
}

/// Per-file attribution entry: who (agent_id) made which edit.
/// Main-thread edits carry `agent_id == None`. Matches TS usage
/// pattern — full `AttributionState` in TS has a ranking +
/// dedup shape; this MVP is a flat log tools append to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttributionEntry {
    pub file_path: String,
    pub timestamp_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AttributionState {
    pub entries: Vec<AttributionEntry>,
}

impl AttributionState {
    pub fn push(&mut self, entry: AttributionEntry) {
        self.entries.push(entry);
    }

    pub fn agents_touching(&self, path: &str) -> Vec<&str> {
        let mut seen = std::collections::BTreeSet::new();
        for e in &self.entries {
            if e.file_path == path {
                if let Some(id) = e.agent_id.as_deref() {
                    seen.insert(id);
                }
            }
        }
        seen.into_iter().collect()
    }
}

/// The authoritative session state held by the actor.
///
/// Immutable from outside the owner task. Consumers get
/// read-only snapshots via [`AppStateHandle::snapshot`] or
/// subscribe to a [`watch::Receiver`] for change streams.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppState {
    /// Map from task id to task state. Task internals stay
    /// opaque (`Value`) until the `TaskState` union is ported
    /// — its variant fields are substantial and not needed by
    /// step 2's contract.
    #[serde(default)]
    pub tasks: HashMap<String, Value>,

    #[serde(default)]
    pub file_history: FileHistoryState,

    #[serde(default)]
    pub attribution: AttributionState,

    /// Tool-use IDs currently executing (parallel-tool gates
    /// + permission UI read this).
    #[serde(default)]
    pub in_progress_tool_uses: HashSet<String>,

    /// Running char total of streamed response this turn.
    #[serde(default)]
    pub response_length: usize,

    /// Agent name → AgentId. Populated by the Agent tool when
    /// `name` is passed (TS `AppStateStore.ts:163`).
    #[serde(default)]
    pub agent_name_registry: HashMap<String, String>,

    /// Permission-scope snapshot consumed by every tool
    /// invocation. Typed aggregate from
    /// `permissions::types::ToolPermissionContext` — the
    /// canonical type the live permission subsystem already
    /// uses.
    #[serde(default)]
    pub tool_permission_context: crate::permissions::types::ToolPermissionContext,

    /// Transcript-injected system messages (hook output, permission
    /// retry notices). Opaque `Value` payloads — the renderer owns
    /// the wire shape. Matches TS `setMessages(appendSystemMessage)`.
    #[serde(default)]
    pub system_messages: Vec<Value>,
}

/// Typed update message variants.
///
/// Discipline: every mutation pattern is an enum variant. The
/// Codex review (2026-04-20) explicitly rejected a generic
/// `Reducer(Box<dyn FnOnce>)` escape hatch — it would weaken
/// audit-trail readability (closures don't `Debug`-format
/// their effect), break serialisation for log replay, and
/// invite ad-hoc state mutations that fragment the state
/// surface. Callers whose mutation doesn't fit an existing
/// variant should **add a variant**.
#[derive(Debug)]
pub enum AppStateUpdate {
    UpsertTask {
        id: String,
        task: Value,
    },
    RemoveTask {
        id: String,
    },
    MarkToolUseInProgress {
        tool_use_id: String,
        in_progress: bool,
    },
    AddResponseChars {
        chars: usize,
    },
    ResetResponseLength,
    AppendFileEdit(FileEdit),
    AppendAttribution(AttributionEntry),
    RegisterAgent {
        name: String,
        agent_id: String,
    },
    /// Injected system-level transcript message (hook output, permission
    /// retry notice, etc.). Opaque `Value` because the message-type
    /// graph lives outside the actor store; the host renders via its
    /// own UI layer. Matches TS `appendSystemMessage`.
    AppendSystemMessage(Value),
}

impl AppStateUpdate {
    fn apply(self, state: &mut AppState) {
        match self {
            Self::UpsertTask { id, task } => {
                state.tasks.insert(id, task);
            }
            Self::RemoveTask { id } => {
                state.tasks.remove(&id);
            }
            Self::MarkToolUseInProgress {
                tool_use_id,
                in_progress,
            } => {
                if in_progress {
                    state.in_progress_tool_uses.insert(tool_use_id);
                } else {
                    state.in_progress_tool_uses.remove(&tool_use_id);
                }
            }
            Self::AddResponseChars { chars } => {
                state.response_length = state.response_length.saturating_add(chars);
            }
            Self::ResetResponseLength => {
                state.response_length = 0;
            }
            Self::AppendFileEdit(edit) => {
                state.file_history.push(edit);
            }
            Self::AppendAttribution(entry) => {
                state.attribution.push(entry);
            }
            Self::RegisterAgent { name, agent_id } => {
                state.agent_name_registry.insert(name, agent_id);
            }
            Self::AppendSystemMessage(msg) => {
                state.system_messages.push(msg);
            }
        }
    }
}

/// Handle callers use to read / write the actor-owned state.
/// Clone freely — each clone routes updates to the same owner
/// task and reads from the same snapshot channel.
#[derive(Clone)]
pub struct AppStateHandle {
    updates: mpsc::UnboundedSender<AppStateUpdate>,
    snapshots: watch::Receiver<Arc<AppState>>,
}

impl AppStateHandle {
    /// Start the owner task and return the handle.
    pub fn spawn() -> Self {
        Self::spawn_with_initial(AppState::default())
    }

    /// Start with a pre-populated state (used for `--resume` /
    /// log-replay paths).
    pub fn spawn_with_initial(initial: AppState) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let (snap_tx, snap_rx) = watch::channel(Arc::new(initial));
        tokio::spawn(owner_loop(rx, snap_tx));
        Self {
            updates: tx,
            snapshots: snap_rx,
        }
    }

    /// Test-only variant returning the owner task handle too,
    /// so the test can `.await` clean shutdown.
    #[cfg(test)]
    fn spawn_for_tests() -> (Self, tokio::task::JoinHandle<()>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let (snap_tx, snap_rx) = watch::channel(Arc::new(AppState::default()));
        let owner = tokio::spawn(owner_loop(rx, snap_tx));
        (
            Self {
                updates: tx,
                snapshots: snap_rx,
            },
            owner,
        )
    }

    /// Read the current snapshot. O(1) — just a cheap `Arc` clone
    /// from the watch channel.
    pub fn snapshot(&self) -> Arc<AppState> {
        self.snapshots.borrow().clone()
    }

    /// Dispatch an update. Returns `Err` only if the owner task
    /// has dropped (shouldn't happen in a healthy session; a
    /// caller should exit gracefully if it does).
    pub fn update(&self, msg: AppStateUpdate) -> Result<(), AppStateShutdown> {
        self.updates.send(msg).map_err(|_| AppStateShutdown)
    }

    /// Subscribe for snapshot updates. The returned receiver
    /// fires every time the owner task publishes a new snapshot.
    pub fn subscribe(&self) -> watch::Receiver<Arc<AppState>> {
        self.snapshots.clone()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct AppStateShutdown;

impl std::fmt::Display for AppStateShutdown {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AppState actor has shut down")
    }
}

impl std::error::Error for AppStateShutdown {}

async fn owner_loop(
    mut updates: mpsc::UnboundedReceiver<AppStateUpdate>,
    snapshots: watch::Sender<Arc<AppState>>,
) {
    while let Some(msg) = updates.recv().await {
        // Optimisation: drain any queued messages and apply them
        // as a batch before publishing a snapshot. Many tools fire
        // 2-3 updates back-to-back (mark-in-progress, add-response-
        // chars, append-file-edit); publishing once per batch keeps
        // subscriber churn low.
        let current = snapshots.borrow().clone();
        let mut next = (*current).clone();
        msg.apply(&mut next);
        while let Ok(next_msg) = updates.try_recv() {
            next_msg.apply(&mut next);
        }
        // Only publish if the watch channel still has receivers.
        let _ = snapshots.send(Arc::new(next));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn settle(_handle: &AppStateHandle) {
        // Force a yield so the owner task can drain + publish.
        tokio::task::yield_now().await;
        for _ in 0..3 {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }

    #[tokio::test]
    async fn spawn_produces_default_snapshot() {
        let handle = AppStateHandle::spawn();
        let snap = handle.snapshot();
        assert!(snap.tasks.is_empty());
        assert_eq!(snap.response_length, 0);
        assert!(snap.in_progress_tool_uses.is_empty());
    }

    #[tokio::test]
    async fn upsert_and_remove_task() {
        let handle = AppStateHandle::spawn();
        handle
            .update(AppStateUpdate::UpsertTask {
                id: "t1".into(),
                task: serde_json::json!({ "type": "in_process_teammate" }),
            })
            .unwrap();
        settle(&handle).await;

        let snap = handle.snapshot();
        assert_eq!(snap.tasks.len(), 1);
        assert_eq!(snap.tasks.get("t1").unwrap()["type"], "in_process_teammate");

        handle
            .update(AppStateUpdate::RemoveTask { id: "t1".into() })
            .unwrap();
        settle(&handle).await;
        assert!(handle.snapshot().tasks.is_empty());
    }

    #[tokio::test]
    async fn mark_tool_use_in_progress_toggles() {
        let handle = AppStateHandle::spawn();
        handle
            .update(AppStateUpdate::MarkToolUseInProgress {
                tool_use_id: "tu1".into(),
                in_progress: true,
            })
            .unwrap();
        settle(&handle).await;
        assert!(handle.snapshot().in_progress_tool_uses.contains("tu1"));

        handle
            .update(AppStateUpdate::MarkToolUseInProgress {
                tool_use_id: "tu1".into(),
                in_progress: false,
            })
            .unwrap();
        settle(&handle).await;
        assert!(!handle.snapshot().in_progress_tool_uses.contains("tu1"));
    }

    #[tokio::test]
    async fn response_length_add_and_reset() {
        let handle = AppStateHandle::spawn();
        handle
            .update(AppStateUpdate::AddResponseChars { chars: 500 })
            .unwrap();
        handle
            .update(AppStateUpdate::AddResponseChars { chars: 250 })
            .unwrap();
        settle(&handle).await;
        assert_eq!(handle.snapshot().response_length, 750);

        handle.update(AppStateUpdate::ResetResponseLength).unwrap();
        settle(&handle).await;
        assert_eq!(handle.snapshot().response_length, 0);
    }

    #[tokio::test]
    async fn file_history_appends() {
        let handle = AppStateHandle::spawn();
        handle
            .update(AppStateUpdate::AppendFileEdit(FileEdit {
                path: "/a.txt".into(),
                timestamp_ms: 100,
                kind: "create".into(),
            }))
            .unwrap();
        handle
            .update(AppStateUpdate::AppendFileEdit(FileEdit {
                path: "/a.txt".into(),
                timestamp_ms: 200,
                kind: "update".into(),
            }))
            .unwrap();
        settle(&handle).await;

        let snap = handle.snapshot();
        assert_eq!(snap.file_history.edits.len(), 2);
        assert_eq!(
            snap.file_history.last_for_path("/a.txt").unwrap().kind,
            "update"
        );
    }

    #[tokio::test]
    async fn attribution_dedups_agents_per_path() {
        let handle = AppStateHandle::spawn();
        for id in ["agent-a", "agent-a", "agent-b"] {
            handle
                .update(AppStateUpdate::AppendAttribution(AttributionEntry {
                    file_path: "/x.rs".into(),
                    timestamp_ms: 0,
                    agent_id: Some(id.into()),
                }))
                .unwrap();
        }
        // Also a main-thread edit (None) to prove it's filtered out.
        handle
            .update(AppStateUpdate::AppendAttribution(AttributionEntry {
                file_path: "/x.rs".into(),
                timestamp_ms: 0,
                agent_id: None,
            }))
            .unwrap();
        settle(&handle).await;

        let snap = handle.snapshot();
        let mut agents = snap.attribution.agents_touching("/x.rs");
        agents.sort();
        assert_eq!(agents, vec!["agent-a", "agent-b"]);
    }

    #[tokio::test]
    async fn register_agent_name() {
        let handle = AppStateHandle::spawn();
        handle
            .update(AppStateUpdate::RegisterAgent {
                name: "researcher".into(),
                agent_id: "a-deadbeef".into(),
            })
            .unwrap();
        settle(&handle).await;
        assert_eq!(
            handle
                .snapshot()
                .agent_name_registry
                .get("researcher")
                .map(String::as_str),
            Some("a-deadbeef"),
        );
    }

    #[tokio::test]
    async fn subscribe_fires_on_updates() {
        let handle = AppStateHandle::spawn();
        let mut rx = handle.subscribe();
        // `mark_unchanged` tells this receiver "I've seen the
        // current value; only flip `changed()` when a NEW one
        // arrives". Without it, the initial state is considered
        // unseen and `changed()` returns immediately with
        // response_length==0 before the owner applies our update.
        rx.mark_unchanged();
        handle
            .update(AppStateUpdate::AddResponseChars { chars: 42 })
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), rx.changed())
            .await
            .expect("timeout waiting for snapshot")
            .unwrap();
        let snap = rx.borrow().clone();
        assert_eq!(snap.response_length, 42);
    }

    #[tokio::test]
    async fn receiver_survives_owner_exit_with_last_snapshot() {
        // Subscriber keeps `watch::Receiver` after all handles
        // drop — they should still see the final published
        // snapshot via `borrow()`. `changed().await` returns
        // `Err` once the watch sender is closed.
        let (handle, owner) = AppStateHandle::spawn_for_tests();
        handle
            .update(AppStateUpdate::AddResponseChars { chars: 777 })
            .unwrap();
        settle(&handle).await;
        let mut rx = handle.subscribe();
        rx.mark_unchanged();
        drop(handle);
        // Owner observes channel close + exits.
        tokio::time::timeout(std::time::Duration::from_secs(1), owner)
            .await
            .expect("owner task didn't exit")
            .unwrap();
        // borrow() still yields the last-published snapshot.
        let snap = rx.borrow().clone();
        assert_eq!(snap.response_length, 777);
        // changed() returns Err because sender is closed.
        let r = rx.changed().await;
        assert!(r.is_err(), "expected Err after sender close, got {r:?}");
    }

    #[tokio::test]
    async fn dropping_all_handles_terminates_owner_task() {
        let (handle, owner) = AppStateHandle::spawn_for_tests();
        drop(handle);
        // Owner task should observe channel close and exit.
        tokio::time::timeout(std::time::Duration::from_secs(1), owner)
            .await
            .expect("owner task didn't exit")
            .unwrap();
    }

    #[tokio::test]
    async fn snapshots_are_arc_cloned() {
        let handle = AppStateHandle::spawn();
        let snap1 = handle.snapshot();
        let snap2 = handle.snapshot();
        // Both snapshots point to the same underlying Arc — the
        // watch::Receiver hands out the current Arc via borrow.
        assert!(Arc::ptr_eq(&snap1, &snap2));
    }

    #[tokio::test]
    async fn batched_updates_produce_single_snapshot_publish_when_fast() {
        // Multiple updates queued before the owner task drains them
        // publish ONE snapshot, not N. We can't observe the publish
        // count directly, but we can confirm that after a burst of
        // 10 updates, the final state reflects ALL of them.
        let handle = AppStateHandle::spawn();
        for i in 0..10 {
            handle
                .update(AppStateUpdate::AddResponseChars { chars: i })
                .unwrap();
        }
        settle(&handle).await;
        assert_eq!(handle.snapshot().response_length, (0..10).sum::<usize>());
    }

    #[test]
    fn app_state_update_debug_human_readable() {
        let e = format!(
            "{:?}",
            AppStateUpdate::MarkToolUseInProgress {
                tool_use_id: "tu-1".into(),
                in_progress: true
            }
        );
        assert!(e.contains("MarkToolUseInProgress"));
        assert!(e.contains("tu-1"));
    }

    #[test]
    fn app_state_roundtrips_through_serde() {
        // Log replay / --resume depends on this.
        let s = AppState {
            tasks: HashMap::from([("t1".into(), serde_json::json!({"type": "x"}))]),
            file_history: FileHistoryState {
                edits: vec![FileEdit {
                    path: "/a".into(),
                    timestamp_ms: 10,
                    kind: "update".into(),
                }],
            },
            attribution: AttributionState {
                entries: vec![AttributionEntry {
                    file_path: "/a".into(),
                    timestamp_ms: 10,
                    agent_id: Some("agent-1".into()),
                }],
            },
            in_progress_tool_uses: HashSet::from(["tu1".into()]),
            response_length: 100,
            agent_name_registry: HashMap::from([("researcher".into(), "a-deadbeef".into())]),
            tool_permission_context: crate::permissions::types::ToolPermissionContext::default(),
            system_messages: vec![serde_json::json!({ "subtype": "hook_output" })],
        };
        let v = serde_json::to_value(&s).unwrap();
        let back: AppState = serde_json::from_value(v).unwrap();
        assert_eq!(back.response_length, 100);
        assert_eq!(back.tasks.len(), 1);
        assert_eq!(back.file_history.edits.len(), 1);
    }
}
