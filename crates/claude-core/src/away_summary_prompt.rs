//! Away-summary prompt builder — function only.
//!
//! Port of TS `services/awaySummary.ts:19` `buildAwaySummaryPrompt`.
//! Fires when the user returns after stepping away from a long
//! session — the Haiku query produces a 1–3 sentence "here's
//! where we were" recap so the user can reorient.
//!
//! The caller (REPL integration that triggers on user idle + return)
//! is not yet wired in Rust; this module exposes the prompt builder
//! so the calling code lands with verbatim TS framing.

/// Build the away-summary user-prompt. Optionally prepends a
/// "Session memory (broader context):" block when session
/// memory is available.
///
/// Port of TS `services/awaySummary.ts:19-23`.
pub fn build_away_summary_prompt(memory: Option<&str>) -> String {
    let memory_block = match memory {
        Some(m) if !m.is_empty() => format!("Session memory (broader context):\n{m}\n\n"),
        _ => String::new(),
    };
    format!(
        "{memory_block}The user stepped away and is coming back. Write exactly 1-3 short \
         sentences. Start by stating the high-level task — what they are building or debugging, \
         not implementation details. Next: the concrete next step. Skip status reports and \
         commit recaps."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_memory_omits_block() {
        let s = build_away_summary_prompt(None);
        assert!(s.starts_with("The user stepped away"));
        assert!(!s.contains("Session memory"));
    }

    #[test]
    fn empty_memory_omits_block() {
        let s = build_away_summary_prompt(Some(""));
        assert!(s.starts_with("The user stepped away"));
        assert!(!s.contains("Session memory"));
    }

    #[test]
    fn memory_prepended_as_block() {
        let s = build_away_summary_prompt(Some("working on auth module"));
        assert!(s.starts_with("Session memory (broader context):\nworking on auth module\n\n"));
        assert!(s.contains("The user stepped away"));
    }

    #[test]
    fn body_anchors_present() {
        let s = build_away_summary_prompt(None);
        assert!(s.contains("1-3 short sentences"));
        assert!(s.contains("high-level task"));
        assert!(s.contains("concrete next step"));
        assert!(s.contains("Skip status reports"));
    }
}
