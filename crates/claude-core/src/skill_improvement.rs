//! Skill-improvement detection + apply.
//!
//! Port of TS `src/utils/hooks/skillImprovement.ts`. Periodically
//! analyzes recent user messages during a skill execution to flag
//! preferences/corrections worth promoting into the skill definition,
//! then re-writes the SKILL.md file when the user accepts the
//! suggestions.
//!
//! Both halves call the secondary (Haiku) model. When no secondary
//! model is registered, both functions return `Ok(None)` so callers
//! can degrade gracefully.

use anyhow::Result;
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

use crate::secondary_model;
use crate::system_prompt_extensions::{
    skill_improvement_apply_prompt, skill_improvement_detection_prompt,
    SKILL_IMPROVEMENT_APPLY_SYSTEM_PROMPT, SKILL_IMPROVEMENT_DETECTION_SYSTEM_PROMPT,
};

/// One suggested update extracted from the LLM detection pass. Mirrors
/// the TS `<updates>[{section, change, reason}]</updates>` payload.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SkillUpdate {
    /// Which step/section to modify, or "new step".
    pub section: String,
    /// What to add/modify.
    pub change: String,
    /// Which user message prompted this suggestion.
    pub reason: String,
}

/// Parse the `<updates>...</updates>` payload TS emits. Tolerates
/// surrounding text — only the JSON inside the tags matters.
fn parse_updates_payload(response: &str) -> Vec<SkillUpdate> {
    let Some(open) = response.find("<updates>") else {
        return Vec::new();
    };
    let after = &response[open + "<updates>".len()..];
    let Some(close) = after.find("</updates>") else {
        return Vec::new();
    };
    let json = &after[..close];
    serde_json::from_str::<Vec<SkillUpdate>>(json.trim()).unwrap_or_default()
}

/// Run the detection pass: look for preferences/corrections in
/// `recent_messages` that should be added to `skill_content`. Returns
/// `Ok(None)` when no secondary model is registered or the LLM returns
/// an empty `<updates>[]</updates>`. Otherwise returns the parsed
/// suggestions.
pub async fn detect_skill_improvements(
    skill_content: &str,
    recent_messages: &str,
    cancel: CancellationToken,
) -> Result<Option<Vec<SkillUpdate>>> {
    let Some(model) = secondary_model::get_global() else {
        return Ok(None);
    };
    let user = skill_improvement_detection_prompt(skill_content, recent_messages);
    let composed = format!("{SKILL_IMPROVEMENT_DETECTION_SYSTEM_PROMPT}\n\n{user}");
    let response = model.summarize(&composed, cancel).await?;
    let updates = parse_updates_payload(&response);
    if updates.is_empty() {
        Ok(None)
    } else {
        Ok(Some(updates))
    }
}

/// Run the apply pass: feed the current SKILL.md + accepted updates
/// to the LLM and parse the `<updated_file>...</updated_file>` payload.
/// Returns `Ok(None)` when no secondary model is registered.
pub async fn apply_skill_improvements(
    current_content: &str,
    updates: &[SkillUpdate],
    cancel: CancellationToken,
) -> Result<Option<String>> {
    let Some(model) = secondary_model::get_global() else {
        return Ok(None);
    };
    let update_list = updates
        .iter()
        .map(|u| {
            format!(
                "- section: {}\n  change: {}\n  reason: {}",
                u.section, u.change, u.reason
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let user = skill_improvement_apply_prompt(current_content, &update_list);
    let composed = format!("{SKILL_IMPROVEMENT_APPLY_SYSTEM_PROMPT}\n\n{user}");
    let response = model.summarize(&composed, cancel).await?;
    Ok(extract_updated_file(&response))
}

fn extract_updated_file(response: &str) -> Option<String> {
    let open = response.find("<updated_file>")?;
    let after = &response[open + "<updated_file>".len()..];
    let close = after.find("</updated_file>")?;
    Some(after[..close].trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_updates_handles_well_formed_payload() {
        let raw = r#"prefix junk
<updates>
[
  {"section": "Phase 2", "change": "ask about energy levels", "reason": "user said 'always ask me about energy'"}
]
</updates>
trailing text"#;
        let v = parse_updates_payload(raw);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].section, "Phase 2");
        assert!(v[0].change.contains("energy levels"));
    }

    #[test]
    fn parse_updates_returns_empty_on_empty_array() {
        let v = parse_updates_payload("<updates>[]</updates>");
        assert!(v.is_empty());
    }

    #[test]
    fn parse_updates_returns_empty_on_missing_tags() {
        assert!(parse_updates_payload("nothing structured here").is_empty());
    }

    #[test]
    fn extract_updated_file_pulls_body() {
        let raw = "<updated_file>\n# New Skill\nBody here\n</updated_file>";
        let body = extract_updated_file(raw).unwrap();
        assert_eq!(body, "# New Skill\nBody here");
    }

    #[tokio::test]
    async fn detect_returns_none_without_secondary_model() {
        // No set_global called — must degrade gracefully.
        let out = detect_skill_improvements("# Skill", "user: hi", CancellationToken::new())
            .await
            .unwrap();
        assert!(out.is_none());
    }

    #[tokio::test]
    async fn apply_returns_none_without_secondary_model() {
        let out = apply_skill_improvements("# Skill", &[], CancellationToken::new())
            .await
            .unwrap();
        assert!(out.is_none());
    }
}
