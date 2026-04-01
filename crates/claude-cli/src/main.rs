use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Parser)]
#[command(name = "claude-rs", about = "Claude Code - AI coding assistant (Rust port)", version)]
pub struct Cli {
    /// Initial prompt (non-interactive mode)
    pub prompt: Option<String>,

    /// Model to use
    #[arg(short, long)]
    pub model: Option<String>,

    /// Verbose output
    #[arg(short, long)]
    pub verbose: bool,

    /// Skip all permission checks (dangerous)
    #[arg(long)]
    pub dangerously_skip_permissions: bool,

    /// Working directory
    #[arg(short = 'C', long = "cd")]
    pub working_dir: Option<PathBuf>,

    /// Resume session by ID
    #[arg(long)]
    pub resume: Option<String>,

    /// Max conversation turns (non-interactive)
    #[arg(long)]
    pub max_turns: Option<u32>,

    /// Append text to system prompt
    #[arg(long)]
    pub append_system_prompt: Option<String>,

    #[command(subcommand)]
    pub command: Option<SubCommand>,
}

#[derive(clap::Subcommand)]
pub enum SubCommand {
    /// Authenticate with Anthropic
    Login,
    /// Remove stored credentials
    Logout,
    /// Show current configuration
    Config,
    /// Start the IDE bridge server
    Server {
        #[arg(long)]
        port: Option<u16>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Set working directory if specified
    if let Some(dir) = &cli.working_dir {
        std::env::set_current_dir(dir)?;
    }

    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            if cli.verbose { "debug" } else { "error" }
        )
        .init();

    // Handle subcommands
    match &cli.command {
        Some(SubCommand::Login) => {
            claude_core::auth::login::login().await?;
            return Ok(());
        }
        Some(SubCommand::Logout) => {
            if let Ok(dir) = claude_core::config::paths::claude_dir() {
                let cred = dir.join(".credentials.json");
                if cred.exists() { let _ = std::fs::remove_file(&cred); }
            }
            if cfg!(target_os = "macos") {
                let user = std::env::var("USER").unwrap_or_default();
                let _ = std::process::Command::new("security")
                    .args(["delete-generic-password", "-a", &user, "-s", "Claude Code-credentials"])
                    .output();
            }
            println!("Logged out successfully.");
            return Ok(());
        }
        Some(SubCommand::Config) => {
            let cwd = std::env::current_dir()?;
            let root = claude_core::config::paths::detect_project_root(&cwd);
            println!("Project root: {}", root.display());
            println!("Config dir: {}", claude_core::config::paths::claude_dir()?.display());
            return Ok(());
        }
        Some(SubCommand::Server { port }) => {
            // Start the IDE bridge server
            let config = claude_core::bridge::types::BridgeConfig {
                port: *port,
                host: "127.0.0.1".to_string(),
                ide: claude_core::bridge::types::IdeType::Other("cli".to_string()),
            };
            let server = claude_core::bridge::server::BridgeServer::new(config);
            tracing::info!("Starting IDE bridge server...");
            server.start().await?;
            return Ok(());
        }
        None => {}
    }

    // Resolve authentication (reads keychain, refreshes OAuth tokens)
    let auth = match claude_core::auth::resolve::resolve_auth().await {
        Ok(auth) => auth,
        Err(e) => {
            eprintln!();
            eprintln!("  \x1b[1mWelcome to Claude Code!\x1b[0m");
            eprintln!();
            eprintln!("  {}", e);
            eprintln!();
            eprintln!("  To get started:");
            eprintln!("  1. Run \x1b[1mclaude login\x1b[0m (if Claude Code is installed)");
            eprintln!("  2. Or set: \x1b[1mexport ANTHROPIC_API_KEY=sk-ant-...\x1b[0m");
            eprintln!();
            std::process::exit(1);
        }
    };

    // Load settings
    let cwd = std::env::current_dir()?;
    let project_root = claude_core::config::paths::detect_project_root(&cwd);

    // Load settings from ~/.claude/settings.json
    let settings = match claude_core::config::paths::user_settings_path() {
        Ok(path) => claude_core::config::settings::Settings::load_from_file(&path),
        Err(_) => claude_core::config::settings::Settings::default(),
    };

    // Build tool registry
    let mut tools = claude_tools::build_default_registry();

    // --- MCP server wiring ---
    // Connect to MCP servers configured in settings.mcpServers
    let mcp_manager = Arc::new(RwLock::new(claude_core::mcp::manager::McpManager::new()));
    if !settings.mcp_servers.is_empty() {
        tracing::info!(
            count = settings.mcp_servers.len(),
            "Connecting to configured MCP servers"
        );
        let mut configs = std::collections::HashMap::new();
        for (name, entry) in &settings.mcp_servers {
            let env = if entry.env.is_empty() {
                None
            } else {
                Some(entry.env.clone())
            };
            let scoped = claude_core::mcp::types::ScopedMcpServerConfig {
                config: claude_core::mcp::types::McpServerConfig::Stdio(
                    claude_core::mcp::types::McpStdioServerConfig {
                        command: entry.command.clone(),
                        args: entry.args.clone(),
                        env,
                    },
                ),
                scope: claude_core::mcp::types::ConfigScope::User,
            };
            configs.insert(name.clone(), scoped);
        }
        let mgr = mcp_manager.read().await;
        let connections = mgr.connect_all(configs).await;
        drop(mgr);

        // Report connection results
        for conn in &connections {
            match &conn.status {
                claude_core::mcp::types::McpConnectionStatus::Connected { .. } => {
                    tracing::info!(server = %conn.name, "MCP server connected");
                }
                claude_core::mcp::types::McpConnectionStatus::Failed { error } => {
                    tracing::warn!(
                        server = %conn.name,
                        error = ?error,
                        "MCP server failed to connect"
                    );
                }
                _ => {}
            }
        }

        // Register MCP tools into the tool registry
        claude_tools::register_mcp_tools(&mut tools, mcp_manager.clone()).await;
    }

    // --- Skill discovery ---
    let skills = claude_core::plugins::skill::discover_skills(&project_root);
    if !skills.is_empty() {
        tracing::info!(count = skills.len(), "Discovered skills");
    }

    let model = cli.model
        .or_else(|| settings.model.clone())
        .unwrap_or_else(|| "claude-sonnet-4-6".into());

    tracing::info!(
        "claude-rs initialized: model={}, tools={}, mcp_servers={}, skills={}, project={}",
        model,
        tools.all().len(),
        settings.mcp_servers.len(),
        skills.len(),
        project_root.display(),
    );

    // Build system prompt
    let tool_descriptions: Vec<(String, String)> = tools.all().iter()
        .map(|t| (t.name().to_string(), format!("Tool: {}", t.name())))
        .collect();
    let system_prompt_values = claude_core::context::system_prompt::build_system_prompt(
        &project_root, &tool_descriptions,
    ).await?;

    // Convert Vec<Value> to Vec<ContentBlock> for the engine
    let system_prompt: Vec<claude_core::types::content::ContentBlock> = system_prompt_values
        .into_iter()
        .filter_map(|v| {
            v.get("text").and_then(|t| t.as_str()).map(|text| {
                claude_core::types::content::ContentBlock::Text {
                    text: text.to_string(),
                }
            })
        })
        .collect();

    // Create API client
    let model_display = model.clone();
    let api_config = claude_core::api::client::ApiConfig {
        model,
        ..Default::default()
    };
    let api_client = claude_core::api::client::ApiClient::new(api_config, auth);

    // Create cancellation token
    let cancel = tokio_util::sync::CancellationToken::new();

    // Get tool definitions for the engine
    let tool_defs = tools.tool_definitions();

    // Create query engine
    let mut query_engine = claude_core::query::engine::QueryEngine::new(
        api_client, system_prompt, tool_defs, cancel.clone(),
    );

    if let Some(max) = cli.max_turns {
        query_engine.set_max_turns(max);
    }

    // Handle --resume: load transcript from a previous session.
    // Supports `--resume <session-id>` or `--resume last` to pick the most recent.
    if let Some(ref session_id_raw) = cli.resume {
        let resolved_id = if session_id_raw.eq_ignore_ascii_case("last") {
            // Find the most recent session by modification time
            let sessions = claude_core::session::manager::SessionManager::list_sessions()?;
            match sessions.first() {
                Some(info) => {
                    tracing::info!("Resolved --resume last to session '{}'", info.id);
                    info.id.clone()
                }
                None => {
                    eprintln!("No previous sessions found to resume.");
                    std::process::exit(1);
                }
            }
        } else {
            session_id_raw.clone()
        };

        let session_mgr = claude_core::session::manager::SessionManager::resume(&resolved_id)?;
        let transcript = session_mgr.storage().load_transcript()?;
        if transcript.is_empty() {
            eprintln!("Warning: session '{}' has no transcript to resume", resolved_id);
        } else {
            tracing::info!("Resuming session '{}' with {} messages", resolved_id, transcript.len());
            eprintln!("Resuming session {} ({} messages)", resolved_id, transcript.len());
            query_engine.load_messages(transcript);
        }
    }

    // Append skill descriptions to the system prompt so the model knows about them
    if !skills.is_empty() {
        let mut skills_text = String::from("\n# Available Skills\n\n");
        skills_text.push_str("The following skills are available for use with the Skill tool:\n\n");
        for skill in &skills {
            skills_text.push_str(&format!("- {}: {}", skill.name, skill.description));
            if let Some(ref hint) = skill.when_to_use {
                skills_text.push_str(&format!(" (use when: {})", hint));
            }
            skills_text.push('\n');
        }
        query_engine.append_system_prompt(skills_text);
    }

    // Append MCP server instructions to the system prompt
    {
        let mgr = mcp_manager.read().await;
        let connections = mgr.connections().await;
        let mut instructions_parts: Vec<String> = Vec::new();
        for conn in &connections {
            if let claude_core::mcp::types::McpConnectionStatus::Connected {
                instructions: Some(ref instr),
                ..
            } = conn.status
            {
                instructions_parts.push(format!("## {}\n{}", conn.name, instr));
            }
        }
        if !instructions_parts.is_empty() {
            query_engine.append_system_prompt(format!(
                "\n# MCP Server Instructions\n\n{}",
                instructions_parts.join("\n\n")
            ));
        }
    }

    // Determine permission mode
    let permission_mode = if cli.dangerously_skip_permissions {
        claude_core::permissions::types::PermissionMode::Bypass
    } else {
        claude_core::permissions::types::PermissionMode::Default
    };

    // Handle non-interactive prompt mode
    if let Some(prompt) = cli.prompt {
        // If using OAuth proxy, delegate to real claude binary
        use std::path::PathBuf;
        use tokio::sync::mpsc;
        use claude_core::permissions::evaluator::evaluate_permission_sync;
        use claude_core::permissions::types::{PermissionDecision, ToolPermissionContext};
        use claude_core::query::engine::TurnResult;
        use claude_core::types::events::StreamEvent;
        use claude_tools::ToolUseContext;

        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let read_file_state = std::sync::Arc::new(std::sync::Mutex::new(
            claude_tools::registry::ReadFileState::new(),
        ));
        let perm_ctx = ToolPermissionContext {
            mode: permission_mode,
            ..Default::default()
        };

        // Check if prompt is a skill invocation (e.g. "/commit fix typo")
        let mut effective_prompt = prompt.clone();
        for skill in &skills {
            if let Some(args) = claude_core::plugins::skill::match_skill(&prompt, skill) {
                let mut content = skill.content.clone();
                if !args.is_empty() {
                    content.push_str(&format!("\n\nArguments: {}", args));
                }
                effective_prompt = content;
                break;
            }
        }

        query_engine.add_user_message(&effective_prompt);

        // Run the agentic loop: prompt -> run_turn -> ToolUse* -> Done
        loop {
            let (stream_tx, mut stream_rx) = mpsc::channel::<StreamEvent>(128);

            // Spawn a task to print streamed text to stdout
            let print_handle = tokio::spawn(async move {
                while let Some(ev) = stream_rx.recv().await {
                    match ev {
                        StreamEvent::TextDelta { text } => {
                            print!("{}", text);
                        }
                        StreamEvent::Done { .. } => {
                            println!();
                        }
                        _ => {}
                    }
                }
            });

            let result = query_engine.run_turn(&stream_tx).await?;
            drop(stream_tx);
            let _ = print_handle.await;

            match result {
                TurnResult::Done(_) => {
                    break;
                }
                TurnResult::ContinueRecovery => {
                    // max_tokens recovery — run again immediately
                    continue;
                }
                TurnResult::ToolUse(tool_uses) => {
                    // Execute each tool, check permissions, feed results back
                    for tool_info in &tool_uses {
                        let is_read_only = tools
                            .get(&tool_info.name)
                            .map(|t| t.is_read_only(&tool_info.input))
                            .unwrap_or(false);

                        let decision = evaluate_permission_sync(
                            &tool_info.name,
                            &tool_info.input,
                            &perm_ctx,
                            is_read_only,
                        );

                        let (result_text, is_error) = match decision {
                            PermissionDecision::Allow | PermissionDecision::Ask { .. } => {
                                // In non-interactive mode, auto-allow (user passed a prompt)
                                let executor = tools.get(&tool_info.name);
                                match executor {
                                    Some(exec) => {
                                        let ctx = ToolUseContext {
                                            working_directory: cwd.clone(),
                                            read_file_state: read_file_state.clone(),
                                        };
                                        match exec.call(&tool_info.input, &ctx, cancel.clone(), None).await {
                                            Ok(data) => {
                                                let text = data.data
                                                    .as_str()
                                                    .unwrap_or(&data.data.to_string())
                                                    .to_string();
                                                (text, data.is_error)
                                            }
                                            Err(e) => (format!("Error: {}", e), true),
                                        }
                                    }
                                    None => (format!("Unknown tool: {}", tool_info.name), true),
                                }
                            }
                            PermissionDecision::Deny { message } => {
                                (format!("Permission denied: {}", message), true)
                            }
                        };

                        query_engine.add_tool_result(&tool_info.id, &result_text, is_error);
                    }
                    // Continue the loop to call run_turn again with the tool results
                }
            }
        }

        // Gracefully disconnect MCP servers
        let mgr = mcp_manager.read().await;
        mgr.disconnect_all().await;

        return Ok(());
    }

    // Interactive TUI mode
    let mut app = claude_tui::app::App::new()?;
    app.set_model_name(&model_display);
    app.set_skills(skills);
    app.run_with_engine(query_engine, tools, cancel.clone(), permission_mode).await?;

    // Gracefully disconnect MCP servers
    let mgr = mcp_manager.read().await;
    mgr.disconnect_all().await;

    Ok(())
}
