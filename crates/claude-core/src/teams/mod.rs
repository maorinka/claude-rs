pub mod backends;
pub mod coordinator;
pub mod mailbox;
pub mod memory;
pub mod permission_sync;
pub mod spawn;
pub mod team_mem_secret_guard;
pub mod types;

// Re-export key types for convenience.
pub use backends::{
    assign_teammate_color, build_inherited_cli_flags, build_inherited_env_vars, format_agent_id,
    generate_unique_teammate_name, parse_agent_id, resolve_teammate_model, sanitize_agent_name,
    sanitize_name, InProcessBackend, TmuxBackend, HIDDEN_SESSION_NAME, PLAN_MODE_REQUIRED_ENV_VAR,
    SWARM_SESSION_NAME, SWARM_VIEW_WINDOW_NAME, TEAMMATE_COLOR_ENV_VAR, TEAMMATE_COMMAND_ENV_VAR,
    TMUX_COMMAND,
};
pub use coordinator::{
    get_coordinator_system_prompt, get_coordinator_user_context, is_coordinator_mode,
    match_session_mode, SessionMode, ASYNC_AGENT_ALLOWED_TOOLS, COORDINATOR_MODE_ALLOWED_TOOLS,
    IN_PROCESS_TEAMMATE_ALLOWED_TOOLS,
};
pub use mailbox::{
    clear_mailbox, create_idle_notification, create_mode_set_request, create_plan_approval_request,
    create_plan_approval_response, create_sandbox_permission_request,
    create_sandbox_permission_response, create_shutdown_approved, create_shutdown_rejected,
    create_shutdown_request, format_teammate_messages, is_idle_notification, is_mode_set_request,
    is_permission_request, is_permission_response, is_plan_approval_request,
    is_plan_approval_response, is_sandbox_permission_request, is_sandbox_permission_response,
    is_shutdown_approved, is_shutdown_rejected, is_shutdown_request,
    is_structured_protocol_message, is_task_assignment, is_team_permission_update,
    mark_message_as_read_by_index, mark_messages_as_read, mark_messages_as_read_by_predicate,
    read_mailbox, read_unread_messages, send_shutdown_request_to_mailbox, write_to_mailbox,
    TeammateMessage, TeammateMessageInput, TEAM_LEAD_NAME,
};
pub use permission_sync::{
    cleanup_old_resolutions, create_permission_request, delete_resolved_permission,
    get_leader_name, is_swarm_worker, is_team_leader, poll_for_response, read_pending_permissions,
    read_resolved_permission, remove_worker_response, resolve_permission,
    send_permission_request_via_mailbox, send_permission_response_via_mailbox,
    send_sandbox_permission_request_via_mailbox, send_sandbox_permission_response_via_mailbox,
    write_permission_request, PermissionDecision, PermissionResolution, PermissionResponse,
    PermissionStatus, ResolvedBy, SwarmPermissionRequest,
};
pub use spawn::{detect_backend, spawn_teammate};
pub use types::{
    AgentColor, AgentStatus, BackendDetectionResult, BackendType, CreatePaneResult, PaneBackend,
    PaneId, PermissionMode, SpawnMode, SpawnOutput, SpawnTeammateConfig, SystemPromptMode, Team,
    TeamAgent, TeamFile, TeamFileMember, TeamStatus, TeammateExecMessage, TeammateExecutor,
    TeammateIdentity, TeammateSpawnConfig, TeammateSpawnResult,
};
