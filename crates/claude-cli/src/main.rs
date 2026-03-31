use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

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
            println!("Login not yet implemented");
            return Ok(());
        }
        Some(SubCommand::Logout) => {
            println!("Logout not yet implemented");
            return Ok(());
        }
        Some(SubCommand::Config) => {
            let cwd = std::env::current_dir()?;
            let root = claude_core::config::paths::detect_project_root(&cwd);
            println!("Project root: {}", root.display());
            println!("Config dir: {}", claude_core::config::paths::claude_dir()?.display());
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

    // Build tool registry
    let tools = claude_tools::build_default_registry();

    let model = cli.model.unwrap_or_else(|| "claude-sonnet-4-6".into());

    tracing::info!(
        "claude-rs initialized: model={}, tools={}, project={}",
        model,
        tools.all().len(),
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
        let perm_ctx = ToolPermissionContext {
            mode: permission_mode,
            ..Default::default()
        };

        query_engine.add_user_message(&prompt);

        // Run the agentic loop: prompt → run_turn → ToolUse* → Done
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

        return Ok(());
    }

    // Interactive TUI mode
    let mut app = claude_tui::app::App::new()?;
    app.set_model_name(&model_display);
    app.run_with_engine(query_engine, tools, cancel, permission_mode).await?;

    Ok(())
}
