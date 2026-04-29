use std::sync::{Arc, OnceLock, RwLock};

use super::types::HookEvent;

const MAX_PENDING_EVENTS: usize = 100;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HookExecutionEvent {
    Started {
        hook_id: String,
        hook_name: String,
        hook_event: String,
    },
    Progress {
        hook_id: String,
        hook_name: String,
        hook_event: String,
        stdout: String,
        stderr: String,
        output: String,
    },
    Response {
        hook_id: String,
        hook_name: String,
        hook_event: String,
        output: String,
        stdout: String,
        stderr: String,
        exit_code: Option<i32>,
        outcome: String,
    },
}

pub struct HookResponseData {
    pub hook_id: String,
    pub hook_name: String,
    pub hook_event: String,
    pub output: String,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub outcome: String,
}

type HookEventHandler = Arc<dyn Fn(HookExecutionEvent) + Send + Sync + 'static>;

#[derive(Default)]
struct HookEventState {
    handler: Option<HookEventHandler>,
    pending: Vec<HookExecutionEvent>,
    all_hook_events_enabled: bool,
}

static HOOK_EVENT_STATE: OnceLock<RwLock<HookEventState>> = OnceLock::new();

fn state() -> &'static RwLock<HookEventState> {
    HOOK_EVENT_STATE.get_or_init(|| RwLock::new(HookEventState::default()))
}

fn should_emit(hook_event: &str, all_hook_events_enabled: bool) -> bool {
    matches!(hook_event, "SessionStart" | "Setup")
        || (all_hook_events_enabled && HookEvent::from_str(hook_event).is_some())
}

pub fn register_hook_event_handler(handler: Option<HookEventHandler>) {
    let pending = {
        let mut guard = state().write().expect("hook event state poisoned");
        guard.handler = handler.clone();
        if handler.is_some() {
            std::mem::take(&mut guard.pending)
        } else {
            Vec::new()
        }
    };

    if let Some(handler) = handler {
        for event in pending {
            handler(event);
        }
    }
}

pub fn set_all_hook_events_enabled(enabled: bool) {
    let mut guard = state().write().expect("hook event state poisoned");
    guard.all_hook_events_enabled = enabled;
}

pub fn clear_hook_event_state() {
    let mut guard = state().write().expect("hook event state poisoned");
    *guard = HookEventState::default();
}

fn emit(event: HookExecutionEvent) {
    let handler = {
        let mut guard = state().write().expect("hook event state poisoned");
        let hook_event = match &event {
            HookExecutionEvent::Started { hook_event, .. }
            | HookExecutionEvent::Progress { hook_event, .. }
            | HookExecutionEvent::Response { hook_event, .. } => hook_event.as_str(),
        };
        if !should_emit(hook_event, guard.all_hook_events_enabled) {
            return;
        }
        if let Some(handler) = guard.handler.clone() {
            Some(handler)
        } else {
            guard.pending.push(event.clone());
            if guard.pending.len() > MAX_PENDING_EVENTS {
                guard.pending.remove(0);
            }
            None
        }
    };

    if let Some(handler) = handler {
        handler(event);
    }
}

pub fn emit_hook_started(hook_id: String, hook_name: String, hook_event: String) {
    emit(HookExecutionEvent::Started {
        hook_id,
        hook_name,
        hook_event,
    });
}

pub fn emit_hook_progress(
    hook_id: String,
    hook_name: String,
    hook_event: String,
    stdout: String,
    stderr: String,
    output: String,
) {
    emit(HookExecutionEvent::Progress {
        hook_id,
        hook_name,
        hook_event,
        stdout,
        stderr,
        output,
    });
}

pub fn emit_hook_response(data: HookResponseData) {
    emit(HookExecutionEvent::Response {
        hook_id: data.hook_id,
        hook_name: data.hook_name,
        hook_event: data.hook_event,
        output: data.output,
        stdout: data.stdout,
        stderr: data.stderr,
        exit_code: data.exit_code,
        outcome: data.outcome,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn collect_events() -> Arc<Mutex<Vec<HookExecutionEvent>>> {
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        register_hook_event_handler(Some(Arc::new(move |event| {
            sink.lock().unwrap().push(event);
        })));
        events
    }

    #[test]
    fn always_emits_session_start_without_all_hook_events() {
        let _guard = TEST_LOCK.lock().unwrap();
        clear_hook_event_state();
        let events = collect_events();

        emit_hook_started(
            "id".into(),
            "SessionStart:startup".into(),
            "SessionStart".into(),
        );
        emit_hook_started("id2".into(), "PreToolUse:Bash".into(), "PreToolUse".into());

        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            HookExecutionEvent::Started { hook_event, .. } if hook_event == "SessionStart"
        ));
        drop(events);
        clear_hook_event_state();
    }

    #[test]
    fn all_hook_events_enabled_emits_tool_hooks() {
        let _guard = TEST_LOCK.lock().unwrap();
        clear_hook_event_state();
        set_all_hook_events_enabled(true);
        let events = collect_events();

        emit_hook_started("id".into(), "PreToolUse:Bash".into(), "PreToolUse".into());

        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            HookExecutionEvent::Started { hook_event, .. } if hook_event == "PreToolUse"
        ));
        drop(events);
        clear_hook_event_state();
    }

    #[test]
    fn pending_events_replay_when_handler_registers() {
        let _guard = TEST_LOCK.lock().unwrap();
        clear_hook_event_state();

        emit_hook_started(
            "id".into(),
            "SessionStart:startup".into(),
            "SessionStart".into(),
        );
        let events = collect_events();

        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            HookExecutionEvent::Started { hook_name, .. } if hook_name == "SessionStart:startup"
        ));
        drop(events);
        clear_hook_event_state();
    }
}
