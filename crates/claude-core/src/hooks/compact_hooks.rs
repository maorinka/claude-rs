use serde_json::{json, Value};

use super::{get_global_runner, HookEvent, HookOutsideReplResult};

#[derive(Debug, Default)]
pub struct PreCompactHookOutput {
    pub custom_instructions: Option<String>,
    pub user_display_message: Option<String>,
}

#[derive(Debug, Default)]
pub struct PostCompactHookOutput {
    pub user_display_message: Option<String>,
}

pub async fn run_pre_compact_hooks(
    trigger: &str,
    custom_instructions: Option<&str>,
) -> PreCompactHookOutput {
    let Some(runner) = get_global_runner() else {
        return PreCompactHookOutput::default();
    };

    let extra = json!({
        "trigger": trigger,
        "custom_instructions": custom_instructions
            .map(|s| Value::String(s.to_string()))
            .unwrap_or(Value::Null),
    });
    let results = runner
        .run_hooks_outside_repl(&HookEvent::PreCompact, extra, None, None)
        .await;

    if results.is_empty() {
        return PreCompactHookOutput::default();
    }

    let successful_outputs = results
        .iter()
        .filter(|result| result.succeeded)
        .map(|result| result.output.trim())
        .filter(|output| !output.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();

    PreCompactHookOutput {
        custom_instructions: if successful_outputs.is_empty() {
            None
        } else {
            Some(successful_outputs.join("\n\n"))
        },
        user_display_message: compact_hook_display_message("PreCompact", &results),
    }
}

pub async fn run_post_compact_hooks(trigger: &str, compact_summary: &str) -> PostCompactHookOutput {
    let Some(runner) = get_global_runner() else {
        return PostCompactHookOutput::default();
    };

    let extra = json!({
        "trigger": trigger,
        "compact_summary": compact_summary,
    });
    let results = runner
        .run_hooks_outside_repl(&HookEvent::PostCompact, extra, None, None)
        .await;

    PostCompactHookOutput {
        user_display_message: compact_hook_display_message("PostCompact", &results),
    }
}

fn compact_hook_display_message(
    event_name: &str,
    results: &[HookOutsideReplResult],
) -> Option<String> {
    if results.is_empty() {
        return None;
    }

    let messages = results
        .iter()
        .map(|result| {
            let output = result.output.trim();
            match (result.succeeded, output.is_empty()) {
                (true, false) => format!(
                    "{event_name} [{}] completed successfully: {output}",
                    result.command
                ),
                (true, true) => {
                    format!("{event_name} [{}] completed successfully", result.command)
                }
                (false, false) => format!("{event_name} [{}] failed: {output}", result.command),
                (false, true) => format!("{event_name} [{}] failed", result.command),
            }
        })
        .collect::<Vec<_>>();

    Some(messages.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_hook_display_message_matches_ts_text() {
        let results = vec![
            HookOutsideReplResult {
                command: "echo ok".to_string(),
                succeeded: true,
                output: " keep this \n".to_string(),
                blocked: false,
                watch_paths: None,
                system_message: None,
            },
            HookOutsideReplResult {
                command: "false".to_string(),
                succeeded: false,
                output: String::new(),
                blocked: false,
                watch_paths: None,
                system_message: None,
            },
        ];

        assert_eq!(
            compact_hook_display_message("PreCompact", &results).as_deref(),
            Some(
                "PreCompact [echo ok] completed successfully: keep this\nPreCompact [false] failed"
            )
        );
    }
}
