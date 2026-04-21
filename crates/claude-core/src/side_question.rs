//! Side-question (`/btw`) wrapper prompt — template only.
//!
//! Port of TS `utils/sideQuestion.ts:61-77`. The `/btw` feature
//! lets the user ask a one-off side question mid-session; TS
//! spawns a lightweight separate agent instance (no tools) with
//! the question wrapped in a `<system-reminder>` that explains
//! the "you are a separate agent, no tools, one-off response"
//! framing.
//!
//! Feature not yet wired — this module exposes the wrapper so
//! the calling path (side-question spawn) lands with the
//! verbatim TS framing when ported.

/// Wrap a user side-question with the `/btw`-framing
/// `<system-reminder>` prelude. Verbatim from
/// `utils/sideQuestion.ts:61-77`.
pub fn wrap_side_question(question: &str) -> String {
    format!(
        "<system-reminder>This is a side question from the user. You must answer this question directly in a single response.\n\
         \n\
         IMPORTANT CONTEXT:\n\
         - You are a separate, lightweight agent spawned to answer this one question\n\
         - The main agent is NOT interrupted - it continues working independently in the background\n\
         - You share the conversation context but are a completely separate instance\n\
         - Do NOT reference being interrupted or what you were \"previously doing\" - that framing is incorrect\n\
         \n\
         CRITICAL CONSTRAINTS:\n\
         - You have NO tools available - you cannot read files, run commands, search, or take any actions\n\
         - This is a one-off response - there will be no follow-up turns\n\
         - You can ONLY provide information based on what you already know from the conversation context\n\
         - NEVER say things like \"Let me try...\", \"I'll now...\", \"Let me check...\", or promise to take any action\n\
         - If you don't know the answer, say so - do not offer to look it up or investigate\n\
         \n\
         Simply answer the question with the information you have.</system-reminder>\n\
         \n\
         {question}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_user_question_with_system_reminder() {
        let wrapped = wrap_side_question("what is 2+2?");
        assert!(wrapped.starts_with("<system-reminder>"));
        assert!(wrapped.contains("</system-reminder>"));
        assert!(wrapped.ends_with("what is 2+2?"));
    }

    #[test]
    fn wrapper_preserves_ts_constraint_phrases() {
        let wrapped = wrap_side_question("hi");
        assert!(wrapped.contains("This is a side question from the user"));
        assert!(wrapped.contains("You are a separate, lightweight agent"));
        assert!(wrapped.contains("You have NO tools available"));
        assert!(wrapped.contains("This is a one-off response"));
    }
}
