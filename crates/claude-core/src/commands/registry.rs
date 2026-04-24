use anyhow::Result;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub enum CommandType {
    Prompt, // Returns text injected as user message
    Action, // Side effects, no message injection
}

pub struct Command {
    pub name: String,
    pub description: String,
    pub command_type: CommandType,
    pub handler: Box<dyn CommandHandler>,
}

pub trait CommandHandler: Send + Sync {
    fn execute(&self, args: &str, ctx: &CommandContext) -> Result<CommandResult>;
}

/// Shared mutable state that slash commands can read and write.
/// Wrapped in `Arc<Mutex<...>>` so multiple subsystems (TUI, engine, commands)
/// can share it.
#[derive(Clone, Debug)]
pub struct SharedCommandState {
    /// Current model name (read/write by /model)
    pub model: String,
    /// Running total of tokens used in this session
    pub total_tokens: u64,
    /// Number of messages in the conversation
    pub message_count: usize,
    /// Session start time
    pub session_start: std::time::Instant,
    /// Current session ID
    pub session_id: String,
    /// Permission mode name
    pub permission_mode: String,
    /// Cost tracker summary (refreshed by the TUI after each turn)
    pub cost_summary: String,
    /// Total API requests made
    pub request_count: u32,
    /// Total cost in USD
    pub total_cost_usd: f64,
    /// Whether fast mode is enabled
    pub fast_mode: bool,
    /// Whether verbose mode is enabled
    pub verbose_mode: bool,
    /// Whether brief mode is enabled
    pub brief_mode: bool,
    /// Effort level: "low", "medium", or "high"
    pub effort_level: String,
    /// Whether the current theme is dark (true) or light (false)
    pub dark_theme: bool,
    /// Theme setting name (e.g. "auto", "dark", "light-daltonized").
    /// Used by the TUI to resolve the active theme via ThemeSetting::from_str.
    pub theme_setting: String,
    /// Context window size in tokens
    pub context_window: u64,
    /// Whether conversation was cleared (signal to the TUI)
    pub clear_requested: bool,
    /// Whether a fork was requested (signal to the TUI)
    pub fork_requested: bool,
    /// Session name set by /rename
    pub session_name: String,
    /// Whether sandbox mode is enabled
    pub sandbox_mode: bool,
    /// Session color name (e.g. "red", "blue")
    pub session_color: String,
    /// Additional working directories added by /add-dir
    pub extra_dirs: Vec<String>,
    /// Per-turn token usage: Vec<(turn_number, input_tokens, output_tokens)>
    pub per_turn_tokens: Vec<(usize, u64, u64)>,
}

impl Default for SharedCommandState {
    fn default() -> Self {
        Self {
            model: "claude-sonnet-4-6".to_string(),
            total_tokens: 0,
            message_count: 0,
            session_start: std::time::Instant::now(),
            session_id: String::new(),
            permission_mode: "default".to_string(),
            cost_summary: String::new(),
            request_count: 0,
            total_cost_usd: 0.0,
            fast_mode: false,
            verbose_mode: false,
            brief_mode: false,
            effort_level: "medium".to_string(),
            dark_theme: true,
            theme_setting: "auto".to_string(),
            context_window: 200_000,
            clear_requested: false,
            fork_requested: false,
            session_name: String::new(),
            sandbox_mode: false,
            session_color: String::new(),
            extra_dirs: Vec::new(),
            per_turn_tokens: Vec::new(),
        }
    }
}

pub struct CommandContext {
    pub working_directory: std::path::PathBuf,
    pub model: String,
    /// Shared mutable state accessible to all commands.
    /// `None` in legacy / test contexts; `Some(...)` when wired to a live session.
    pub shared: Option<Arc<Mutex<SharedCommandState>>>,
}

pub enum CommandResult {
    Message(String), // Inject as user message
    Action(String),  // Print output, no message
    Error(String),   // Error message
}

pub struct CommandRegistry {
    commands: HashMap<String, Command>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self {
            commands: HashMap::new(),
        }
    }

    pub fn register(&mut self, cmd: Command) {
        self.commands.insert(cmd.name.clone(), cmd);
    }

    pub fn get(&self, name: &str) -> Option<&Command> {
        self.commands.get(name)
    }

    pub fn all(&self) -> Vec<&Command> {
        self.commands.values().collect()
    }

    pub fn search(&self, query: &str) -> Vec<&Command> {
        self.commands
            .values()
            .filter(|c| {
                c.name.contains(query)
                    || c.description.to_lowercase().contains(&query.to_lowercase())
            })
            .collect()
    }
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}
