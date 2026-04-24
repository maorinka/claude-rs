//! Main entry point for spawning teammates.
//!
//! This module provides `spawn_teammate()` which orchestrates the full
//! teammate creation flow: backend detection, name dedup, agent ID
//! generation, pane/process creation, mailbox seeding, and team file
//! registration.

use anyhow::Result;
use tracing::{debug, info, warn};

use super::backends::*;
use super::mailbox::{self, read_team_file, write_team_file, TeammateMessageInput, TEAM_LEAD_NAME};
use super::types::*;

// ---------------------------------------------------------------------------
// Backend detection
// ---------------------------------------------------------------------------

/// Detect which backend to use for spawning teammates.
///
/// Priority:
/// 1. If `CLAUDE_CODE_TEAMMATE_BACKEND` env var is set, use that.
/// 2. If inside tmux, use tmux.
/// 3. If tmux is available, use tmux (external session).
/// 4. Fall back to in-process.
pub async fn detect_backend() -> BackendDetectionResult {
    // Check env override.
    if let Ok(backend_env) = std::env::var("CLAUDE_CODE_TEAMMATE_BACKEND") {
        match backend_env.as_str() {
            "tmux" => {
                return BackendDetectionResult {
                    backend_type: BackendType::Tmux,
                    is_native: TmuxBackend::is_inside_tmux(),
                    needs_it2_setup: false,
                }
            }
            "in-process" => {
                return BackendDetectionResult {
                    backend_type: BackendType::InProcess,
                    is_native: true,
                    needs_it2_setup: false,
                }
            }
            _ => {}
        }
    }

    // Check if in-process is forced.
    if is_in_process_forced() {
        return BackendDetectionResult {
            backend_type: BackendType::InProcess,
            is_native: true,
            needs_it2_setup: false,
        };
    }

    // Check tmux.
    if TmuxBackend::is_inside_tmux() {
        return BackendDetectionResult {
            backend_type: BackendType::Tmux,
            is_native: true,
            needs_it2_setup: false,
        };
    }

    if TmuxBackend::is_available().await {
        return BackendDetectionResult {
            backend_type: BackendType::Tmux,
            is_native: false,
            needs_it2_setup: false,
        };
    }

    // Fall back to in-process.
    BackendDetectionResult {
        backend_type: BackendType::InProcess,
        is_native: true,
        needs_it2_setup: false,
    }
}

/// Check if in-process mode is forced via env var.
fn is_in_process_forced() -> bool {
    std::env::var("CLAUDE_CODE_IN_PROCESS_TEAMMATES")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Main spawn function
// ---------------------------------------------------------------------------

/// Spawn a teammate using the appropriate backend.
///
/// This is the main entry point that orchestrates:
/// 1. Backend detection
/// 2. Name deduplication
/// 3. Agent ID generation
/// 4. Pane/process creation
/// 5. Mailbox seeding with initial prompt
/// 6. Team file registration
pub async fn spawn_teammate(
    config: &SpawnTeammateConfig,
    team_name: &str,
    parent_session_id: &str,
    leader_model: Option<&str>,
    permission_mode: Option<&PermissionMode>,
) -> Result<SpawnOutput> {
    let detection = detect_backend().await;

    debug!(
        "[spawn] Using backend {:?} for teammate '{}'",
        detection.backend_type, config.name
    );

    match detection.backend_type {
        BackendType::Tmux => {
            spawn_with_tmux(
                config,
                team_name,
                parent_session_id,
                leader_model,
                permission_mode,
                detection.is_native,
            )
            .await
        }
        BackendType::InProcess => {
            spawn_in_process(config, team_name, parent_session_id, leader_model).await
        }
        BackendType::ITerm2 => {
            // iTerm2 backend not yet implemented in Rust; fall back to tmux.
            warn!("[spawn] iTerm2 backend not implemented; falling back to tmux");
            spawn_with_tmux(
                config,
                team_name,
                parent_session_id,
                leader_model,
                permission_mode,
                false,
            )
            .await
        }
    }
}

// ---------------------------------------------------------------------------
// Tmux spawn
// ---------------------------------------------------------------------------

async fn spawn_with_tmux(
    config: &SpawnTeammateConfig,
    team_name: &str,
    parent_session_id: &str,
    leader_model: Option<&str>,
    permission_mode: Option<&PermissionMode>,
    inside_tmux: bool,
) -> Result<SpawnOutput> {
    let model = resolve_teammate_model(config.model.as_deref(), leader_model, None);

    // Generate unique name if duplicate exists.
    let unique_name = generate_unique_teammate_name(&config.name, team_name).await;
    let sanitized_name = sanitize_agent_name(&unique_name);
    let teammate_id = format_agent_id(&sanitized_name, team_name);
    let working_dir = config.cwd.as_deref().unwrap_or(".");

    // Assign color based on current team size.
    let team_file = read_team_file(team_name).await;
    let color_index = team_file.as_ref().map_or(0, |tf| tf.members.len());
    let teammate_color = assign_teammate_color(color_index);

    // Create the pane.
    let backend = TmuxBackend::new();
    let create_result = if inside_tmux {
        TmuxBackend::create_teammate_pane_with_leader(&sanitized_name, teammate_color).await?
    } else {
        backend
            .create_teammate_pane_external(&sanitized_name, teammate_color)
            .await?
    };

    // Enable pane border status on first teammate when inside tmux.
    if create_result.is_first_teammate && inside_tmux {
        TmuxBackend::enable_pane_border_status_impl(None, false).await;
    }

    // Build the spawn command.
    let binary_path = get_teammate_command();

    let mut teammate_args = vec![
        format!("--agent-id '{}'", shell_escape_val(&teammate_id)),
        format!("--agent-name '{}'", shell_escape_val(&sanitized_name)),
        format!("--team-name '{}'", shell_escape_val(team_name)),
        format!("--agent-color '{}'", teammate_color),
        format!(
            "--parent-session-id '{}'",
            shell_escape_val(parent_session_id)
        ),
    ];

    if config.plan_mode_required {
        teammate_args.push("--plan-mode-required".to_string());
    }
    if let Some(ref agent_type) = config.agent_type {
        teammate_args.push(format!("--agent-type '{}'", shell_escape_val(agent_type)));
    }

    let teammate_args_str = teammate_args.join(" ");

    // Build inherited CLI flags.
    let mut inherited_flags = build_inherited_cli_flags(
        config.plan_mode_required,
        permission_mode,
        None, // model_override -- we handle it separately below
        None,
        &[],
        None,
        None,
    );

    // Add/replace model flag.
    if !model.is_empty() {
        // Remove any existing --model from inherited flags.
        let parts: Vec<&str> = inherited_flags.split_whitespace().collect();
        let mut filtered = Vec::new();
        let mut skip_next = false;
        for part in &parts {
            if skip_next {
                skip_next = false;
                continue;
            }
            if *part == "--model" {
                skip_next = true;
                continue;
            }
            if part.starts_with("--model=") {
                continue;
            }
            filtered.push(*part);
        }
        inherited_flags = filtered.join(" ");
        let model_flag = format!("--model '{}'", shell_escape_val(&model));
        if inherited_flags.is_empty() {
            inherited_flags = model_flag;
        } else {
            inherited_flags = format!("{} {}", inherited_flags, model_flag);
        }
    }

    let flags_str = if inherited_flags.is_empty() {
        String::new()
    } else {
        format!(" {}", inherited_flags)
    };

    let env_str = build_inherited_env_vars();
    let spawn_command = format!(
        "cd '{}' && env {} '{}' {}{}",
        shell_escape_val(working_dir),
        env_str,
        shell_escape_val(&binary_path),
        teammate_args_str,
        flags_str,
    );

    // Send the command to the new pane.
    TmuxBackend::send_command_to_pane_impl(&create_result.pane_id, &spawn_command, !inside_tmux)
        .await?;

    let session_name = if inside_tmux {
        "current".to_string()
    } else {
        SWARM_SESSION_NAME.to_string()
    };
    let window_name = if inside_tmux {
        "current".to_string()
    } else {
        SWARM_VIEW_WINDOW_NAME.to_string()
    };

    // Register in team file.
    if let Some(mut tf) = read_team_file(team_name).await {
        tf.members.push(TeamFileMember {
            agent_id: teammate_id.clone(),
            name: sanitized_name.clone(),
            agent_type: config.agent_type.clone(),
            model: Some(model.clone()),
            prompt: Some(config.prompt.clone()),
            color: Some(teammate_color.to_string()),
            plan_mode_required: config.plan_mode_required,
            joined_at: Some(chrono::Utc::now().timestamp_millis() as u64),
            tmux_pane_id: Some(create_result.pane_id.clone()),
            cwd: Some(working_dir.to_string()),
            subscriptions: vec![],
            backend_type: Some(BackendType::Tmux),
        });
        let _ = write_team_file(team_name, &tf).await;
    }

    // Send initial prompt via mailbox.
    let _ = mailbox::write_to_mailbox(
        &sanitized_name,
        TeammateMessageInput {
            from: TEAM_LEAD_NAME.to_string(),
            text: config.prompt.clone(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            color: None,
            summary: None,
        },
        Some(team_name),
    )
    .await;

    info!(
        "[spawn] Spawned tmux teammate '{}' (id={}, pane={})",
        sanitized_name, teammate_id, create_result.pane_id
    );

    Ok(SpawnOutput {
        teammate_id: teammate_id.clone(),
        agent_id: teammate_id,
        agent_type: config.agent_type.clone(),
        model: Some(model),
        name: sanitized_name,
        color: Some(teammate_color.to_string()),
        tmux_session_name: session_name,
        tmux_window_name: window_name,
        tmux_pane_id: create_result.pane_id,
        team_name: Some(team_name.to_string()),
        is_splitpane: config.use_splitpane,
        plan_mode_required: config.plan_mode_required,
    })
}

// ---------------------------------------------------------------------------
// In-process spawn
// ---------------------------------------------------------------------------

async fn spawn_in_process(
    config: &SpawnTeammateConfig,
    team_name: &str,
    parent_session_id: &str,
    leader_model: Option<&str>,
) -> Result<SpawnOutput> {
    let model = resolve_teammate_model(config.model.as_deref(), leader_model, None);

    let unique_name = generate_unique_teammate_name(&config.name, team_name).await;
    let sanitized_name = sanitize_agent_name(&unique_name);
    let teammate_id = format_agent_id(&sanitized_name, team_name);

    let backend = InProcessBackend::new();
    let spawn_config = TeammateSpawnConfig {
        name: sanitized_name.clone(),
        team_name: team_name.to_string(),
        color: None,
        plan_mode_required: config.plan_mode_required,
        prompt: config.prompt.clone(),
        cwd: config.cwd.clone().unwrap_or_else(|| ".".to_string()),
        model: Some(model.clone()),
        system_prompt: None,
        system_prompt_mode: SystemPromptMode::Default,
        worktree_path: None,
        parent_session_id: parent_session_id.to_string(),
        permissions: vec![],
        allow_permission_prompts: false,
        agent_type: config.agent_type.clone(),
        description: config.description.clone(),
    };

    let result = backend.spawn(&spawn_config).await?;

    // Register in team file.
    if let Some(mut tf) = read_team_file(team_name).await {
        tf.members.push(TeamFileMember {
            agent_id: teammate_id.clone(),
            name: sanitized_name.clone(),
            agent_type: config.agent_type.clone(),
            model: Some(model.clone()),
            prompt: Some(config.prompt.clone()),
            color: None,
            plan_mode_required: config.plan_mode_required,
            joined_at: Some(chrono::Utc::now().timestamp_millis() as u64),
            tmux_pane_id: None,
            cwd: config.cwd.clone(),
            subscriptions: vec![],
            backend_type: Some(BackendType::InProcess),
        });
        let _ = write_team_file(team_name, &tf).await;
    }

    info!(
        "[spawn] Spawned in-process teammate '{}' (id={})",
        sanitized_name, teammate_id
    );

    Ok(SpawnOutput {
        teammate_id: teammate_id.clone(),
        agent_id: teammate_id,
        agent_type: config.agent_type.clone(),
        model: Some(model),
        name: sanitized_name,
        color: None,
        tmux_session_name: String::new(),
        tmux_window_name: String::new(),
        tmux_pane_id: result.pane_id.unwrap_or_default(),
        team_name: Some(team_name.to_string()),
        is_splitpane: false,
        plan_mode_required: config.plan_mode_required,
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Get the command to use for spawning teammate processes.
fn get_teammate_command() -> String {
    if let Ok(cmd) = std::env::var(TEAMMATE_COMMAND_ENV_VAR) {
        if !cmd.is_empty() {
            return cmd;
        }
    }
    std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "claude-rs".to_string())
}

fn shell_escape_val(s: &str) -> String {
    s.replace('\'', "'\\''")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_detect_backend_defaults() {
        // In a test environment without tmux, should fall back to in-process.
        // Unless TMUX env var is set.
        let result = detect_backend().await;
        // Can't assert specific backend since it depends on environment,
        // but the function should not panic.
        assert!(
            result.backend_type == BackendType::Tmux
                || result.backend_type == BackendType::InProcess
        );
    }

    #[test]
    fn test_is_in_process_forced() {
        // Without env var set, should be false.
        std::env::remove_var("CLAUDE_CODE_IN_PROCESS_TEAMMATES");
        assert!(!is_in_process_forced());
    }

    #[test]
    fn test_get_teammate_command() {
        // When env var is not set, should return current exe or fallback.
        std::env::remove_var(TEAMMATE_COMMAND_ENV_VAR);
        let cmd = get_teammate_command();
        assert!(!cmd.is_empty());
    }
}
