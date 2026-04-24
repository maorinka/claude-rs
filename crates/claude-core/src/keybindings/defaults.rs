//! Port of `src/keybindings/defaultBindings.ts`.
//!
//! Feature-gated entries (KAIROS, QUICK_SEARCH, TERMINAL_PANEL,
//! MESSAGE_ACTIONS, VOICE_MODE) are always included here — callers that
//! want strict TS parity can filter by context name at load time. Most
//! sections produce enormous amounts of dead text and the gain of gating
//! each one doesn't outweigh the readability cost.

use super::parser::KeybindingBlock;
use std::collections::BTreeMap;

fn block(context: &str, bindings: &[(&str, &str)]) -> KeybindingBlock {
    let mut m = BTreeMap::new();
    for (k, v) in bindings {
        m.insert((*k).to_string(), (*v).to_string());
    }
    KeybindingBlock {
        context: context.to_string(),
        bindings: m,
    }
}

/// The default keybinding configuration.
///
/// On Windows without VT mode, `shift+tab` is unreliable — TS substitutes
/// `meta+m` there. We keep `shift+tab` unconditional in this port; callers
/// on legacy Windows Terminal can override via user keybindings.json.
pub fn default_bindings() -> Vec<KeybindingBlock> {
    vec![
        block(
            "Global",
            &[
                ("ctrl+c", "app:interrupt"),
                ("ctrl+d", "app:exit"),
                ("ctrl+l", "app:redraw"),
                ("ctrl+t", "app:toggleTodos"),
                ("ctrl+o", "app:toggleTranscript"),
                ("ctrl+shift+b", "app:toggleBrief"),
                ("ctrl+shift+o", "app:toggleTeammatePreview"),
                ("ctrl+r", "history:search"),
                ("ctrl+shift+f", "app:globalSearch"),
                ("cmd+shift+f", "app:globalSearch"),
                ("ctrl+shift+p", "app:quickOpen"),
                ("cmd+shift+p", "app:quickOpen"),
                ("meta+j", "app:toggleTerminal"),
            ],
        ),
        block(
            "Chat",
            &[
                ("escape", "chat:cancel"),
                ("ctrl+x ctrl+k", "chat:killAgents"),
                ("shift+tab", "chat:cycleMode"),
                ("meta+p", "chat:modelPicker"),
                ("meta+o", "chat:fastMode"),
                ("meta+t", "chat:thinkingToggle"),
                ("enter", "chat:submit"),
                ("up", "history:previous"),
                ("down", "history:next"),
                ("ctrl+_", "chat:undo"),
                ("ctrl+shift+-", "chat:undo"),
                ("ctrl+x ctrl+e", "chat:externalEditor"),
                ("ctrl+g", "chat:externalEditor"),
                ("ctrl+s", "chat:stash"),
                ("ctrl+v", "chat:imagePaste"),
                ("shift+up", "chat:messageActions"),
                ("space", "voice:pushToTalk"),
            ],
        ),
        block(
            "Autocomplete",
            &[
                ("tab", "autocomplete:accept"),
                ("escape", "autocomplete:dismiss"),
                ("up", "autocomplete:previous"),
                ("down", "autocomplete:next"),
            ],
        ),
        block(
            "Settings",
            &[
                ("escape", "confirm:no"),
                ("up", "select:previous"),
                ("down", "select:next"),
                ("k", "select:previous"),
                ("j", "select:next"),
                ("ctrl+p", "select:previous"),
                ("ctrl+n", "select:next"),
                ("space", "select:accept"),
                ("enter", "settings:close"),
                ("/", "settings:search"),
                ("r", "settings:retry"),
            ],
        ),
        block(
            "Confirmation",
            &[
                ("y", "confirm:yes"),
                ("n", "confirm:no"),
                ("enter", "confirm:yes"),
                ("escape", "confirm:no"),
                ("up", "confirm:previous"),
                ("down", "confirm:next"),
                ("tab", "confirm:nextField"),
                ("space", "confirm:toggle"),
                ("shift+tab", "confirm:cycleMode"),
                ("ctrl+e", "confirm:toggleExplanation"),
                ("ctrl+d", "permission:toggleDebug"),
            ],
        ),
        block(
            "Tabs",
            &[
                ("tab", "tabs:next"),
                ("shift+tab", "tabs:previous"),
                ("right", "tabs:next"),
                ("left", "tabs:previous"),
            ],
        ),
        block(
            "Transcript",
            &[
                ("ctrl+e", "transcript:toggleShowAll"),
                ("ctrl+c", "transcript:exit"),
                ("escape", "transcript:exit"),
                ("q", "transcript:exit"),
            ],
        ),
        block(
            "HistorySearch",
            &[
                ("ctrl+r", "historySearch:next"),
                ("escape", "historySearch:accept"),
                ("tab", "historySearch:accept"),
                ("ctrl+c", "historySearch:cancel"),
                ("enter", "historySearch:execute"),
            ],
        ),
        block("Task", &[("ctrl+b", "task:background")]),
        block(
            "ThemePicker",
            &[("ctrl+t", "theme:toggleSyntaxHighlighting")],
        ),
        block(
            "Scroll",
            &[
                ("pageup", "scroll:pageUp"),
                ("pagedown", "scroll:pageDown"),
                ("ctrl+home", "scroll:top"),
                ("ctrl+end", "scroll:bottom"),
                ("ctrl+shift+c", "selection:copy"),
                ("cmd+c", "selection:copy"),
            ],
        ),
        block("Help", &[("escape", "help:dismiss")]),
        block(
            "Attachments",
            &[
                ("right", "attachments:next"),
                ("left", "attachments:previous"),
                ("backspace", "attachments:remove"),
                ("delete", "attachments:remove"),
                ("down", "attachments:exit"),
                ("escape", "attachments:exit"),
            ],
        ),
        block(
            "Footer",
            &[
                ("up", "footer:up"),
                ("ctrl+p", "footer:up"),
                ("down", "footer:down"),
                ("ctrl+n", "footer:down"),
                ("right", "footer:next"),
                ("left", "footer:previous"),
                ("enter", "footer:openSelected"),
                ("escape", "footer:clearSelection"),
            ],
        ),
        block(
            "DiffDialog",
            &[
                ("escape", "diff:dismiss"),
                ("left", "diff:previousSource"),
                ("right", "diff:nextSource"),
                ("up", "diff:previousFile"),
                ("down", "diff:nextFile"),
                ("enter", "diff:viewDetails"),
            ],
        ),
        block(
            "ModelPicker",
            &[
                ("left", "modelPicker:decreaseEffort"),
                ("right", "modelPicker:increaseEffort"),
            ],
        ),
        block(
            "Select",
            &[
                ("up", "select:previous"),
                ("down", "select:next"),
                ("j", "select:next"),
                ("k", "select:previous"),
                ("ctrl+n", "select:next"),
                ("ctrl+p", "select:previous"),
                ("enter", "select:accept"),
                ("escape", "select:cancel"),
            ],
        ),
        block(
            "Plugin",
            &[("space", "plugin:toggle"), ("i", "plugin:install")],
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_includes_global_interrupt() {
        let b = default_bindings();
        let global = b.iter().find(|bl| bl.context == "Global").unwrap();
        assert_eq!(
            global.bindings.get("ctrl+c").map(String::as_str),
            Some("app:interrupt")
        );
        assert_eq!(
            global.bindings.get("ctrl+d").map(String::as_str),
            Some("app:exit")
        );
    }

    #[test]
    fn default_includes_chat_mode_cycle() {
        let b = default_bindings();
        let chat = b.iter().find(|bl| bl.context == "Chat").unwrap();
        assert_eq!(
            chat.bindings.get("shift+tab").map(String::as_str),
            Some("chat:cycleMode")
        );
        assert_eq!(
            chat.bindings.get("enter").map(String::as_str),
            Some("chat:submit")
        );
    }

    #[test]
    fn context_coverage() {
        let b = default_bindings();
        let names: std::collections::HashSet<_> = b.iter().map(|bl| bl.context.as_str()).collect();
        for required in &[
            "Global",
            "Chat",
            "Autocomplete",
            "Settings",
            "Confirmation",
            "Tabs",
            "Transcript",
            "HistorySearch",
            "Scroll",
            "Help",
            "Select",
        ] {
            assert!(names.contains(required), "missing context {}", required);
        }
    }
}
