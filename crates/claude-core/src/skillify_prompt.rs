//! `/skillify` bundled-skill prompt.
//!
//! Port of TS `src/skills/bundled/skillify.ts`. Captures the
//! current session's repeatable process as a reusable skill by
//! analyzing session memory + user messages, then interviewing
//! the user across four rounds. The Rust port currently ships
//! only the prompt text; the AskUserQuestion / SessionMemory
//! content wiring is not yet implemented.
//!
//! TS template slots (all filled via string `.replace()`):
//! - `{{userDescriptionBlock}}` — header caption from user args
//! - `{{sessionMemory}}` — SessionMemory content (fallback
//!   "No session memory available.")
//! - `{{userMessages}}` — extracted user messages since the last
//!   compact boundary, joined by `\n\n---\n\n`

const SKILLIFY_PROMPT_TEMPLATE: &str = include_str!("prompts/skillify.md");

/// Fallback body for the `{{sessionMemory}}` slot. Port of TS
/// `skillify.ts:181` default.
pub const SKILLIFY_NO_SESSION_MEMORY: &str = "No session memory available.";

/// Build the caption line that fills `{{userDescriptionBlock}}`.
/// Port of TS `skillify.ts:186-188`.
pub fn skillify_user_description_block(args: &str) -> String {
    if args.is_empty() {
        String::new()
    } else {
        format!("The user described this process as: \"{args}\"")
    }
}

/// Join extracted user messages with the TS `\n\n---\n\n`
/// separator. Port of TS `skillify.ts:191`.
pub fn skillify_join_user_messages(messages: &[String]) -> String {
    messages.join("\n\n---\n\n")
}

/// Fill every template slot and return the rendered prompt.
/// `user_messages` is pre-joined (caller uses
/// [`skillify_join_user_messages`] or equivalent). Pass an empty
/// string for `args` to omit the caption.
///
/// Port of TS `skillify.ts:190-192` (the three-call
/// `.replace()` chain).
pub fn skillify_prompt(args: &str, session_memory: &str, user_messages: &str) -> String {
    let memory = if session_memory.is_empty() {
        SKILLIFY_NO_SESSION_MEMORY
    } else {
        session_memory
    };
    SKILLIFY_PROMPT_TEMPLATE
        .replace("{{sessionMemory}}", memory)
        .replace("{{userMessages}}", user_messages)
        .replace(
            "{{userDescriptionBlock}}",
            &skillify_user_description_block(args),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_has_four_round_interview() {
        let p = skillify_prompt("", "", "");
        assert!(p.contains("**Round 1: High level confirmation**"));
        assert!(p.contains("**Round 2: More details**"));
        assert!(p.contains("**Round 3: Breaking down each step**"));
        assert!(p.contains("**Round 4: Final questions**"));
    }

    #[test]
    fn description_block_rendered_when_args_present() {
        let p = skillify_prompt("cherry-pick flow", "", "");
        assert!(p.contains("The user described this process as: \"cherry-pick flow\""));
    }

    #[test]
    fn description_block_blank_when_args_empty() {
        let p = skillify_prompt("", "some memory", "some msgs");
        assert!(!p.contains("The user described this process as"));
        // Header kept stable: `# Skillify ` with a trailing space when
        // the block is empty — matches TS template.
        assert!(p.contains("# Skillify "));
    }

    #[test]
    fn session_memory_substituted_or_defaulted() {
        let filled = skillify_prompt("", "<mem>CUSTOM</mem>", "");
        assert!(filled.contains("<mem>CUSTOM</mem>"));
        assert!(!filled.contains("No session memory available."));

        let defaulted = skillify_prompt("", "", "");
        assert!(defaulted.contains("No session memory available."));
    }

    #[test]
    fn user_messages_inlined_verbatim() {
        let msgs = skillify_join_user_messages(&[
            "first".to_string(),
            "second".to_string(),
            "third".to_string(),
        ]);
        let p = skillify_prompt("", "", &msgs);
        assert!(p.contains("first\n\n---\n\nsecond\n\n---\n\nthird"));
    }

    #[test]
    fn no_unsubstituted_slots_after_render() {
        let p = skillify_prompt("x", "y", "z");
        assert!(!p.contains("{{sessionMemory}}"));
        assert!(!p.contains("{{userMessages}}"));
        assert!(!p.contains("{{userDescriptionBlock}}"));
    }

    #[test]
    fn teaches_askuserquestion_discipline() {
        let p = skillify_prompt("", "", "");
        assert!(p.contains("Use AskUserQuestion for ALL questions"));
        assert!(p.contains("freeform \"Other\" option"));
    }
}
