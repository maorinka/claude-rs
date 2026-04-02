use std::io::{self, Stdout};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{
    self, Event as CrosstermEvent, KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind,
};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{cursor, execute};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout, Rect};
// Paragraph no longer used — layout renders directly to buffer
use ratatui::Terminal;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use claude_core::permissions::evaluator::evaluate_permission;
use claude_core::permissions::types::{PermissionDecision, PermissionMode, ToolPermissionContext};
use claude_core::query::engine::{QueryEngine, ToolUseInfo, TurnResult};
use claude_core::types::events::{StreamEvent, ToolResultData};
use claude_tools::{ToolRegistry, ToolUseContext};

use claude_core::commands::builtin::build_default_commands;
use claude_core::commands::registry::{
    CommandContext, CommandRegistry, CommandResult, SharedCommandState,
};
use claude_core::cost::tracker::CostTracker;
use claude_core::plugins::skill;
use claude_core::plugins::types::Skill;

use std::sync::{Arc, Mutex};

use crate::theme::{detect_theme, Theme};
use crate::widgets::ask_user_dialog::AskUserDialog;
use crate::widgets::command_picker::{CommandPicker, CommandPickerEntry, CommandPickerWidget};
use crate::widgets::message_list::{MessageEntry, MessageList, MessageListWidget};
use crate::widgets::model_picker::{ModelPicker, ModelPickerWidget};
use crate::widgets::permission_dialog::PermissionDialog;
use crate::widgets::prompt_input::{InputAction, PromptInput};
use crate::widgets::spinner::{SpinnerMode, SpinnerState};

/// Token budget warning thresholds.
const TOKEN_WARNING_THRESHOLD: f64 = 0.80; // Yellow warning at 80%
const TOKEN_CRITICAL_THRESHOLD: f64 = 0.95; // Red warning at 95%

pub enum AppEvent {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Resize(u16, u16),
    Tick,
    SpinnerTick,
    Quit,
    Stream(StreamEvent),
    SubmitPrompt(String),
    PermissionResponse(String),
    /// Fired after all tool results have been added — tells the main loop to
    /// call `engine.run_turn()` and handle the next result (which may be
    /// another round of tool use or a final answer).
    ContinueTurn,
    /// Fired when a tool result signals `awaiting_input: true`.
    /// Contains the tool-use id (to feed the reply back), the question text,
    /// and an optional list of preset options.
    ShowAskUserDialog {
        tool_use_id: String,
        question: String,
        options: Vec<String>,
    },
    /// Fired when the user submits their answer in the AskUser dialog.
    AskUserResponse(String),
    /// Fired when a background `engine.run_turn()` completes.
    TurnComplete(Result<TurnResult, anyhow::Error>),
}

/// Commands sent to the dedicated engine task via a channel.
/// The engine owns the `QueryEngine` exclusively; all interaction goes through
/// these non-blocking sends so the event loop never awaits the engine.
enum EngineCommand {
    AddUserMessage(String),
    AddToolResult {
        id: String,
        content: String,
        is_error: bool,
    },
    RunTurn(mpsc::Sender<StreamEvent>),
    LoadMessages(Vec<serde_json::Value>),
}

/// Pending tool that needs permission before execution.
struct PendingTool {
    info: ToolUseInfo,
}

/// Result of executing a slash command.
enum CommandAction {
    /// Inject the text as a user prompt message.
    Prompt(String),
    /// Display the text as a system/action message (no prompt injection).
    Display(String),
}

pub struct App {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    theme: Theme,
    spinner: SpinnerState,
    should_quit: bool,
    message_list: MessageList,
    prompt: PromptInput,
    permission_dialog: Option<PermissionDialog>,
    /// Dialog shown when AskUserQuestionTool is awaiting user input.
    ask_user_dialog: Option<AskUserDialog>,
    /// True while the engine is processing (prevents double-submit)
    engine_busy: bool,
    /// Model name for display in the header
    model_name: String,
    /// Running total of tokens used in this session
    total_tokens: u64,
    /// Cost tracker for the session -- accumulates usage from StreamEvent::UsageUpdate
    cost_tracker: CostTracker,
    /// Command picker overlay (shown on `/` at start of input)
    command_picker: CommandPicker,
    /// Model picker overlay (shown on `/model`)
    model_picker: ModelPicker,
    /// Command registry for slash commands
    command_registry: CommandRegistry,
    /// Discovered skills
    skills: Vec<Skill>,
    /// Shared mutable state for slash commands (persistent across calls)
    shared_state: Arc<Mutex<SharedCommandState>>,
    /// Session start time for duration display
    _session_start: std::time::Instant,
    /// Context window size for token budget warnings
    context_window: u64,
    /// Viewport height (updated on render, used for page-up/down)
    viewport_height: u16,
}

impl App {
    pub fn new() -> Result<Self> {
        let backend = CrosstermBackend::new(io::stdout());
        let terminal = Terminal::new(backend)?;
        let command_registry = build_default_commands();
        Ok(Self {
            terminal,
            theme: detect_theme(),
            spinner: SpinnerState::new(),
            should_quit: false,
            message_list: MessageList::new(),
            prompt: PromptInput::new(),
            permission_dialog: None,
            ask_user_dialog: None,
            engine_busy: false,
            model_name: "claude-sonnet-4-6".to_string(),
            total_tokens: 0,
            cost_tracker: CostTracker::new("claude-sonnet-4-6"),
            command_picker: CommandPicker::new(),
            model_picker: ModelPicker::new(),
            command_registry,
            skills: Vec::new(),
            shared_state: Arc::new(Mutex::new(SharedCommandState::default())),
            _session_start: std::time::Instant::now(),
            context_window: 200_000,
            viewport_height: 24,
        })
    }

    /// Add the welcome header matching TS LogoV2 — shows version, model, and cwd.
    fn push_welcome_header(&mut self) {
        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| ".".to_string());
        // Shorten home dir to ~
        let home = dirs::home_dir()
            .map(|h| h.display().to_string())
            .unwrap_or_default();
        let display_cwd = if !home.is_empty() && cwd.starts_with(&home) {
            format!("~{}", &cwd[home.len()..])
        } else {
            cwd
        };

        self.message_list.push(MessageEntry::Logo {
            model: self.model_name.clone(),
            cwd: display_cwd,
        });
    }

    /// Set the model name displayed in the header and cost tracker.
    pub fn set_model_name(&mut self, name: &str) {
        self.model_name = name.to_string();
        self.cost_tracker = CostTracker::new(name);
        if let Ok(mut state) = self.shared_state.lock() {
            state.model = name.to_string();
        }
    }

    /// Set the permission mode name for slash command display.
    pub fn set_permission_mode(&mut self, mode: &str) {
        if let Ok(mut state) = self.shared_state.lock() {
            state.permission_mode = mode.to_string();
        }
    }

    /// Set the session ID for slash command display.
    pub fn set_session_id(&mut self, id: &str) {
        if let Ok(mut state) = self.shared_state.lock() {
            state.session_id = id.to_string();
        }
    }

    /// Set discovered skills for the command picker.
    pub fn set_skills(&mut self, skills: Vec<Skill>) {
        self.skills = skills;
    }

    /// Build command picker entries from the command registry and discovered skills.
    fn build_picker_entries(&self) -> Vec<CommandPickerEntry> {
        let mut entries: Vec<CommandPickerEntry> = self
            .command_registry
            .all()
            .iter()
            .map(|cmd| CommandPickerEntry {
                name: cmd.name.clone(),
                description: cmd.description.clone(),
            })
            .collect();
        // Sort commands alphabetically
        entries.sort_by(|a, b| a.name.cmp(&b.name));

        // Add user-invocable skills
        for skill in &self.skills {
            if skill.user_invocable {
                entries.push(CommandPickerEntry {
                    name: skill.name.clone(),
                    description: skill.description.clone(),
                });
            }
        }

        entries
    }

    /// Try to execute a slash command or skill from user input.
    /// Returns Some(text_to_inject) for Prompt-type commands, or handles
    /// Action-type commands directly. Returns None if the input is not a command.
    fn try_execute_command(&mut self, input: &str) -> Option<CommandAction> {
        let trimmed = input.trim();
        if !trimmed.starts_with('/') {
            return None;
        }

        let without_slash = &trimmed[1..];
        let (cmd_name, args) = match without_slash.split_once(' ') {
            Some((name, rest)) => (name, rest),
            None => (without_slash, ""),
        };

        // Check command registry first
        if let Some(cmd) = self.command_registry.get(cmd_name) {
            // Sync shared state with current app state before executing
            if let Ok(mut state) = self.shared_state.lock() {
                state.cost_summary = self.cost_tracker.detailed_summary();
                state.model = self.model_name.clone();
                state.total_tokens = self.total_tokens;
                state.request_count = self.cost_tracker.request_count();
                state.total_cost_usd = self.cost_tracker.total_cost_usd();
                state.message_count = self.message_list.messages().len();
            }

            let ctx = CommandContext {
                working_directory: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
                model: self.model_name.clone(),
                shared: Some(self.shared_state.clone()),
            };
            let result = cmd.handler.execute(args, &ctx);

            // React to state mutations made by the command
            if let Ok(state) = self.shared_state.lock() {
                // Model change
                if state.model != self.model_name {
                    self.model_name = state.model.clone();
                    self.cost_tracker = CostTracker::new(&state.model);
                }
                // Theme change
                if state.dark_theme
                    != (matches!(self.theme.fg, ratatui::style::Color::Rgb(255, 255, 255)))
                {
                    self.theme = if state.dark_theme {
                        crate::theme::dark_theme()
                    } else {
                        crate::theme::light_theme()
                    };
                }
            }

            match result {
                Ok(CommandResult::Message(text)) => {
                    return Some(CommandAction::Prompt(text));
                }
                Ok(CommandResult::Action(text)) => {
                    return Some(CommandAction::Display(text));
                }
                Ok(CommandResult::Error(text)) => {
                    return Some(CommandAction::Display(format!("Error: {}", text)));
                }
                Err(e) => {
                    return Some(CommandAction::Display(format!("Error: {}", e)));
                }
            }
        }

        // Check skills
        for s in &self.skills {
            if let Some(args) = skill::match_skill(trimmed, s) {
                let mut content = s.content.clone();
                if !args.is_empty() {
                    content.push_str(&format!("\n\nArguments: {}", args));
                }
                return Some(CommandAction::Prompt(content));
            }
        }

        None
    }

    /// Original standalone run loop (no engine). Kept for backwards compatibility.
    pub async fn run(&mut self) -> Result<()> {
        terminal::enable_raw_mode()?;
        execute!(
            io::stdout(),
            EnterAlternateScreen,
            cursor::Hide
        )?;

        let (tx, mut rx) = mpsc::channel::<AppEvent>(100);

        // Spawn input reader
        let tx_input = tx.clone();
        tokio::spawn(async move {
            loop {
                if event::poll(Duration::from_millis(16)).unwrap_or(false) {
                    if let Ok(evt) = event::read() {
                        let app_evt = match evt {
                            CrosstermEvent::Key(k)
                                if k.kind == crossterm::event::KeyEventKind::Press =>
                            {
                                Some(AppEvent::Key(k))
                            }
                            CrosstermEvent::Key(_) => None, // Ignore Release/Repeat on Windows
                            CrosstermEvent::Resize(w, h) => Some(AppEvent::Resize(w, h)),
                            CrosstermEvent::Mouse(m) => Some(AppEvent::Mouse(m)),
                            _ => None,
                        };
                        if let Some(e) = app_evt {
                            if tx_input.send(e).await.is_err() {
                                break;
                            }
                        }
                    }
                }
            }
        });

        // Spawn render tick (60fps)
        let tx_tick = tx.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(16));
            loop {
                interval.tick().await;
                if tx_tick.send(AppEvent::Tick).await.is_err() {
                    break;
                }
            }
        });

        // Spawn spinner tick (50ms)
        let tx_spinner = tx.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(50));
            loop {
                interval.tick().await;
                if tx_spinner.send(AppEvent::SpinnerTick).await.is_err() {
                    break;
                }
            }
        });

        while !self.should_quit {
            if let Some(event) = rx.recv().await {
                match event {
                    AppEvent::Tick => self.render()?,
                    AppEvent::SpinnerTick => self.spinner.advance(),
                    AppEvent::Key(k) => self.handle_key_standalone(k),
                    AppEvent::Resize(_, _) => self.render()?,
                    AppEvent::Mouse(m) => self.handle_mouse(m),
                    AppEvent::Quit => self.should_quit = true,
                    _ => {}
                }
            }
        }

        terminal::disable_raw_mode()?;
        execute!(
            io::stdout(),
            LeaveAlternateScreen,
            cursor::Show
        )?;
        Ok(())
    }

    /// Run the TUI wired to the QueryEngine.
    pub async fn run_with_engine(
        &mut self,
        engine: QueryEngine,
        tools: ToolRegistry,
        cancel: CancellationToken,
        permission_mode: PermissionMode,
    ) -> Result<()> {
        terminal::enable_raw_mode()?;
        execute!(
            io::stdout(),
            EnterAlternateScreen,
            cursor::Hide
        )?;

        let (tx, mut rx) = mpsc::channel::<AppEvent>(256);

        // Channel-based engine: the engine lives in its own task and receives
        // commands via `engine_tx`.  Sends are non-blocking so the event loop
        // never awaits anything that depends on the engine.
        let (engine_tx, mut engine_rx) = mpsc::channel::<EngineCommand>(32);
        {
            let app_tx = tx.clone();
            let mut engine = engine;
            tokio::spawn(async move {
                while let Some(cmd) = engine_rx.recv().await {
                    match cmd {
                        EngineCommand::AddUserMessage(text) => {
                            engine.add_user_message(&text);
                        }
                        EngineCommand::AddToolResult { id, content, is_error } => {
                            engine.add_tool_result(&id, &content, is_error);
                        }
                        EngineCommand::RunTurn(stream_tx) => {
                            let result = engine.run_turn(&stream_tx).await;
                            let _ = app_tx
                                .send(AppEvent::TurnComplete(result.map_err(Into::into)))
                                .await;
                        }
                        EngineCommand::LoadMessages(msgs) => {
                            engine.load_messages(msgs);
                        }
                    }
                }
            });
        }

        // Spawn input reader
        let tx_input = tx.clone();
        tokio::spawn(async move {
            loop {
                if event::poll(Duration::from_millis(16)).unwrap_or(false) {
                    if let Ok(evt) = event::read() {
                        let app_evt = match evt {
                            CrosstermEvent::Key(k)
                                if k.kind == crossterm::event::KeyEventKind::Press =>
                            {
                                Some(AppEvent::Key(k))
                            }
                            CrosstermEvent::Key(_) => None, // Ignore Release/Repeat on Windows
                            CrosstermEvent::Resize(w, h) => Some(AppEvent::Resize(w, h)),
                            CrosstermEvent::Mouse(m) => Some(AppEvent::Mouse(m)),
                            _ => None,
                        };
                        if let Some(e) = app_evt {
                            if tx_input.send(e).await.is_err() {
                                break;
                            }
                        }
                    }
                }
            }
        });

        // Spawn render tick (60fps)
        let tx_tick = tx.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(16));
            loop {
                interval.tick().await;
                if tx_tick.send(AppEvent::Tick).await.is_err() {
                    break;
                }
            }
        });

        // Spawn spinner tick (50ms)
        let tx_spinner = tx.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(50));
            loop {
                interval.tick().await;
                if tx_spinner.send(AppEvent::SpinnerTick).await.is_err() {
                    break;
                }
            }
        });

        let mut cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        // Remember the original directory so ExitWorktree can restore it.
        let original_cwd = cwd.clone();

        // Shared read-file state for staleness tracking across tool calls
        let read_file_state = std::sync::Arc::new(std::sync::Mutex::new(
            claude_tools::registry::ReadFileState::new(),
        ));

        // Set permission mode in shared state
        {
            let mode_str = match &permission_mode {
                PermissionMode::BypassPermissions => "bypass",
                PermissionMode::Auto => "auto",
                PermissionMode::Default => "default",
                PermissionMode::AcceptEdits => "accept-edits",
                PermissionMode::Plan => "plan",
                PermissionMode::DontAsk => "dont-ask",
                PermissionMode::Bubble => "bubble",
            };
            if let Ok(mut state) = self.shared_state.lock() {
                state.permission_mode = mode_str.to_string();
                // Detect initial theme from current app state
                state.dark_theme =
                    matches!(self.theme.fg, ratatui::style::Color::Rgb(255, 255, 255));
            }
        }

        let perm_ctx = ToolPermissionContext {
            mode: permission_mode,
            ..Default::default()
        };

        // Tools waiting for permission resolution
        let mut pending_tools: Vec<PendingTool> = Vec::new();
        // Current index into pending_tools when walking through permission dialogs
        let mut pending_tool_index: usize = 0;

        // Show welcome header (matching TS LogoV2)
        self.push_welcome_header();

        // Main event loop
        while !self.should_quit {
            let Some(event) = rx.recv().await else {
                break;
            };

            match event {
                AppEvent::Tick => {
                    self.render()?;
                }
                AppEvent::SpinnerTick => {
                    self.spinner.advance();
                }
                AppEvent::Resize(_, _) => {
                    self.render()?;
                }
                AppEvent::Mouse(m) => {
                    self.handle_mouse(m);
                }
                AppEvent::Quit => {
                    self.should_quit = true;
                }
                AppEvent::Key(k) => {
                    // Ctrl+C / Ctrl+D always quits
                    if matches!(
                        (k.modifiers, k.code),
                        (KeyModifiers::CONTROL, KeyCode::Char('c'))
                            | (KeyModifiers::CONTROL, KeyCode::Char('d'))
                    ) {
                        cancel.cancel();
                        self.should_quit = true;
                        continue;
                    }

                    if self.ask_user_dialog.is_some() {
                        // Route keys to ask-user input dialog
                        if let Some(ref mut dialog) = self.ask_user_dialog {
                            if let Some(answer) = dialog.handle_key(k) {
                                let _ = tx.send(AppEvent::AskUserResponse(answer)).await;
                            }
                        }
                    } else if self.permission_dialog.is_some() {
                        // Route keys to permission dialog
                        match k.code {
                            KeyCode::Tab | KeyCode::Right => {
                                if let Some(ref mut dialog) = self.permission_dialog {
                                    dialog.next_button();
                                }
                            }
                            KeyCode::BackTab | KeyCode::Left => {
                                if let Some(ref mut dialog) = self.permission_dialog {
                                    dialog.prev_button();
                                }
                            }
                            KeyCode::Enter => {
                                let response = self
                                    .permission_dialog
                                    .as_ref()
                                    .map(|d| d.selected().to_string())
                                    .unwrap_or_else(|| "deny".to_string());
                                let _ = tx.send(AppEvent::PermissionResponse(response)).await;
                            }
                            _ => {}
                        }
                    } else if self.model_picker.visible {
                        // Route keys to model picker: ↑↓ model, ←→ effort, Enter confirm, Esc cancel
                        match k.code {
                            KeyCode::Esc => {
                                self.model_picker.close();
                            }
                            KeyCode::Up => self.model_picker.prev(),
                            KeyCode::Down => self.model_picker.next(),
                            KeyCode::Left => self.model_picker.effort_left(),
                            KeyCode::Right => self.model_picker.effort_right(),
                            KeyCode::Enter => {
                                if let Some((model, effort)) = self.model_picker.confirm() {
                                    let display = claude_core::commands::builtin::render_model_name(&model);
                                    self.model_name = model.clone();
                                    if let Ok(mut state) = self.shared_state.lock() {
                                        state.model = model;
                                    }
                                    let effort_str = effort
                                        .map(|e| format!(" with {} effort", e.as_str()))
                                        .unwrap_or_default();
                                    self.message_list.push(MessageEntry::System {
                                        text: format!("Set model to {}{}", display, effort_str),
                                    });
                                }
                            }
                            _ => {}
                        }
                    } else if self.command_picker.visible {
                        // Route keys to command picker
                        match k.code {
                            KeyCode::Esc => {
                                self.command_picker.close();
                            }
                            KeyCode::Up => {
                                self.command_picker.prev();
                            }
                            KeyCode::Down | KeyCode::Tab => {
                                self.command_picker.next();
                            }
                            KeyCode::Enter => {
                                if let Some(name) = self.command_picker.selected_name() {
                                    let cmd_text = format!("/{}", name);
                                    self.command_picker.close();
                                    let _ = tx.send(AppEvent::SubmitPrompt(cmd_text)).await;
                                } else {
                                    self.command_picker.close();
                                }
                            }
                            KeyCode::Backspace => {
                                self.prompt.handle_key(k);
                                let new_text = self.prompt.text().to_string();
                                if !new_text.starts_with('/') {
                                    self.command_picker.close();
                                } else {
                                    let query = new_text.strip_prefix('/').unwrap_or("");
                                    self.command_picker.set_query(query);
                                }
                            }
                            KeyCode::Char(_)
                                if k.modifiers.is_empty() || k.modifiers == KeyModifiers::SHIFT =>
                            {
                                // Forward keystroke to prompt so text updates
                                self.prompt.handle_key(k);
                                // Read updated text and filter picker
                                let full_text = self.prompt.text().to_string();
                                if let Some(q) = full_text.strip_prefix('/') {
                                    self.command_picker.set_query(q);
                                } else {
                                    self.command_picker.close();
                                }
                            }
                            _ => {}
                        }
                    } else {
                        // Scroll keyboard shortcuts (take priority over prompt)
                        match (k.modifiers, k.code) {
                            (KeyModifiers::NONE, KeyCode::PageUp) => {
                                self.message_list.page_up(self.viewport_height as usize);
                                continue;
                            }
                            (KeyModifiers::NONE, KeyCode::PageDown) => {
                                self.message_list.page_down(self.viewport_height as usize);
                                continue;
                            }
                            (KeyModifiers::NONE, KeyCode::Home) => {
                                self.message_list.scroll_to_top();
                                continue;
                            }
                            (KeyModifiers::NONE, KeyCode::End) => {
                                self.message_list.scroll_to_bottom();
                                continue;
                            }
                            (KeyModifiers::CONTROL, KeyCode::Up) => {
                                self.message_list.scroll_up(1);
                                continue;
                            }
                            (KeyModifiers::CONTROL, KeyCode::Down) => {
                                self.message_list.scroll_down(1);
                                continue;
                            }
                            // Ctrl+O: toggle thinking block visibility (matches TS)
                            (KeyModifiers::CONTROL, KeyCode::Char('o')) => {
                                self.message_list.toggle_thinking();
                                continue;
                            }
                            // Ctrl+L: clear screen (redraw)
                            (KeyModifiers::CONTROL, KeyCode::Char('l')) => {
                                let _ = self.terminal.clear();
                                continue;
                            }
                            _ => {}
                        }

                        // Route keys to prompt input
                        match self.prompt.handle_key(k) {
                            InputAction::Submit(text) => {
                                let _ = tx.send(AppEvent::SubmitPrompt(text)).await;
                            }
                            InputAction::None => {
                                let text = self.prompt.text().to_string();
                                if text.starts_with('/') {
                                    if !self.command_picker.visible {
                                        // Open picker on first `/`
                                        let entries = self.build_picker_entries();
                                        self.command_picker.open(entries);
                                    }
                                    // Update filter with text after `/`
                                    let query = &text[1..];
                                    self.command_picker.set_query(query);
                                } else if self.command_picker.visible {
                                    // Close picker if `/` was deleted
                                    self.command_picker.close();
                                }
                            }
                        }
                    }
                }
                AppEvent::SubmitPrompt(text) => {
                    if self.engine_busy || text.trim().is_empty() {
                        continue;
                    }

                    // Intercept /model to open interactive picker
                    if text.trim() == "/model" || text.trim().starts_with("/model ") {
                        let args = text.trim().strip_prefix("/model").unwrap_or("").trim();
                        if args.is_empty() {
                            // No args: open interactive picker
                            self.prompt.clear();
                            self.model_picker.open(&self.model_name);
                            continue;
                        }
                        // Has args: set model directly
                        let new_model = claude_core::commands::builtin::parse_user_specified_model(args);
                        let display = claude_core::commands::builtin::render_model_name(&new_model);
                        self.model_name = new_model.clone();
                        if let Some(ref shared) = Some(&self.shared_state) {
                            if let Ok(mut state) = shared.lock() {
                                state.model = new_model;
                            }
                        }
                        self.message_list.push(MessageEntry::System {
                            text: format!("Set model to {}", display),
                        });
                        continue;
                    }

                    // Try to handle as a slash command first
                    if text.starts_with('/') {
                        match self.try_execute_command(&text) {
                            Some(CommandAction::Display(output)) => {
                                // Action-type command: show output as system message
                                self.message_list
                                    .push(MessageEntry::System { text: output });

                                // Handle side effects from stateful commands
                                if let Ok(mut state) = self.shared_state.lock() {
                                    if state.clear_requested {
                                        state.clear_requested = false;
                                        let _ = engine_tx.send(EngineCommand::LoadMessages(Vec::new())).await;
                                        self.message_list = MessageList::new();
                                        self.total_tokens = 0;
                                        self.cost_tracker = CostTracker::new(&self.model_name);
                                        self.message_list.push(MessageEntry::System {
                                            text: "Conversation history cleared.".to_string(),
                                        });
                                    }
                                    if state.fork_requested {
                                        state.fork_requested = false;
                                        // Fork: create a new session storage with a copy
                                        // of current messages. The engine continues with
                                        // its existing message history.
                                    }
                                    // Sync brief mode with the tool global state
                                    claude_tools::brief_tool::set_brief_mode(state.brief_mode);
                                }

                                continue;
                            }
                            Some(CommandAction::Prompt(prompt_text)) => {
                                // Prompt-type command: inject as user message
                                self.message_list
                                    .push(MessageEntry::User { text: text.clone() });
                                let _ = engine_tx.send(EngineCommand::AddUserMessage(prompt_text)).await;
                                self.engine_busy = true;
                                self.spinner.start(SpinnerMode::Thinking);

                                let (stream_tx, stream_rx) = mpsc::channel::<StreamEvent>(128);
                                let tx_forward = tx.clone();
                                tokio::spawn(async move {
                                    let mut stream_rx = stream_rx;
                                    while let Some(ev) = stream_rx.recv().await {
                                        if tx_forward.send(AppEvent::Stream(ev)).await.is_err() {
                                            break;
                                        }
                                    }
                                });

                                let _ = engine_tx.send(EngineCommand::RunTurn(stream_tx)).await;
                                continue;
                            }
                            None => {
                                // Not a known command, treat as regular input
                            }
                        }
                    }

                    // `! command` prefix: run shell command and show output
                    // (matches TS behavior where `!` runs a command in the session)
                    if text.starts_with("! ")
                        || text.starts_with("!")
                            && text.len() > 1
                            && text.chars().nth(1) != Some('!')
                    {
                        let cmd = text
                            .strip_prefix("! ")
                            .or_else(|| text.strip_prefix("!"))
                            .unwrap_or("");
                        if !cmd.trim().is_empty() {
                            self.message_list
                                .push(MessageEntry::User { text: text.clone() });
                            let output =
                                std::process::Command::new("sh").arg("-c").arg(cmd).output();
                            match output {
                                Ok(out) => {
                                    let stdout = String::from_utf8_lossy(&out.stdout);
                                    let stderr = String::from_utf8_lossy(&out.stderr);
                                    let combined = if stderr.is_empty() {
                                        stdout.to_string()
                                    } else if stdout.is_empty() {
                                        stderr.to_string()
                                    } else {
                                        format!("{}\n{}", stdout, stderr)
                                    };
                                    self.message_list
                                        .push(MessageEntry::System { text: combined });
                                }
                                Err(e) => {
                                    self.message_list.push(MessageEntry::System {
                                        text: format!("Error running command: {}", e),
                                    });
                                }
                            }
                            continue;
                        }
                    }

                    // Regular user message
                    // Add user message to display
                    self.message_list
                        .push(MessageEntry::User { text: text.clone() });

                    // Add to engine (non-blocking channel send)
                    let _ = engine_tx.send(EngineCommand::AddUserMessage(text)).await;
                    self.engine_busy = true;
                    self.spinner.start(SpinnerMode::Thinking);

                    // Run turn — stream events are forwarded to the main event loop
                    let (stream_tx, mut stream_rx) = mpsc::channel::<StreamEvent>(128);
                    let tx_forward = tx.clone();

                    // Spawn a task to forward stream events to the main event loop
                    tokio::spawn(async move {
                        while let Some(ev) = stream_rx.recv().await {
                            if tx_forward.send(AppEvent::Stream(ev)).await.is_err() {
                                break;
                            }
                        }
                    });

                    // Tell the engine task to run the turn; the engine task will
                    // send TurnComplete back via app_tx when done.
                    let _ = engine_tx.send(EngineCommand::RunTurn(stream_tx)).await;
                }
                AppEvent::Stream(stream_event) => {
                    self.handle_stream_event(stream_event);
                }
                AppEvent::PermissionResponse(response) => {
                    self.permission_dialog = None;

                    if pending_tool_index == 0 || pending_tool_index > pending_tools.len() {
                        continue;
                    }
                    let tool_idx = pending_tool_index - 1; // We already advanced past it

                    match response.as_str() {
                        "allow" | "always" => {
                            // Execute this tool
                            let info = &pending_tools[tool_idx].info;
                            self.spinner.start(SpinnerMode::Tool {
                                name: info.name.clone(),
                            });
                            let tool_result = execute_tool(
                                &tools,
                                &info.name,
                                &info.input,
                                &cwd,
                                cancel.clone(),
                                read_file_state.clone(),
                            )
                            .await;
                            self.spinner.stop();

                            // Update working directory when entering/exiting a worktree.
                            // EnterWorktree returns { worktreePath } on success — switch cwd.
                            // ExitWorktree success means we return to the original directory.
                            if let Ok(ref data) = tool_result {
                                if !data.is_error {
                                    match info.name.as_str() {
                                        "EnterWorktree" => {
                                            if let Some(path) = data.data["worktreePath"].as_str() {
                                                cwd = PathBuf::from(path);
                                                tracing::info!("Session cwd switched to worktree: {}", path);
                                            }
                                        }
                                        "ExitWorktree" => {
                                            cwd = original_cwd.clone();
                                            tracing::info!(
                                                "Session cwd restored to: {}",
                                                original_cwd.display()
                                            );
                                        }
                                        _ => {}
                                    }
                                }
                            }

                            // Detect AskUserQuestionTool awaiting_input signal
                            let awaiting = tool_result
                                .as_ref()
                                .ok()
                                .and_then(|d| d.data["awaiting_input"].as_bool())
                                .unwrap_or(false);

                            if awaiting {
                                // Don't feed result back yet — show dialog instead.
                                let question = tool_result
                                    .as_ref()
                                    .ok()
                                    .and_then(|d| d.data["question"].as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let opts: Vec<String> = tool_result
                                    .as_ref()
                                    .ok()
                                    .and_then(|d| d.data["options"].as_array())
                                    .map(|arr| {
                                        arr.iter()
                                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                                            .collect()
                                    })
                                    .unwrap_or_default();
                                let tool_id = info.id.clone();
                                let tx2 = tx.clone();
                                tokio::spawn(async move {
                                    let _ = tx2
                                        .send(AppEvent::ShowAskUserDialog {
                                            tool_use_id: tool_id,
                                            question,
                                            options: opts,
                                        })
                                        .await;
                                });
                            } else {
                                let (result_text, is_error) = match &tool_result {
                                    Ok(data) => (
                                        data.data
                                            .as_str()
                                            .unwrap_or(&data.data.to_string())
                                            .to_string(),
                                        data.is_error,
                                    ),
                                    Err(e) => (format!("Error: {}", e), true),
                                };
                                let _ = engine_tx.send(EngineCommand::AddToolResult {
                                    id: info.id.clone(),
                                    content: result_text.clone(),
                                    is_error,
                                }).await;
                                self.message_list.push(MessageEntry::ToolResult {
                                    name: info.name.clone(),
                                    output: truncate_result(&result_text),
                                    is_error,
                                });

                                // Check next tool or continue turn
                                if pending_tool_index < pending_tools.len() {
                                    self.check_next_tool_permission(
                                        &pending_tools,
                                        &mut pending_tool_index,
                                        &perm_ctx,
                                        &tools,
                                        &tx,
                                    );
                                } else {
                                    // All tools done — fire ContinueTurn to re-enter the engine
                                    let tx2 = tx.clone();
                                    tokio::spawn(async move {
                                        let _ = tx2.send(AppEvent::ContinueTurn).await;
                                    });
                                }
                            }
                        }
                        "deny" => {
                            let info = &pending_tools[tool_idx].info;
                            let _ = engine_tx.send(EngineCommand::AddToolResult {
                                id: info.id.clone(),
                                content: "Permission denied by user".to_string(),
                                is_error: true,
                            }).await;
                            self.message_list.push(MessageEntry::ToolResult {
                                name: info.name.clone(),
                                output: "Permission denied".to_string(),
                                is_error: true,
                            });

                            // Check next tool or continue turn
                            if pending_tool_index < pending_tools.len() {
                                self.check_next_tool_permission(
                                    &pending_tools,
                                    &mut pending_tool_index,
                                    &perm_ctx,
                                    &tools,
                                    &tx,
                                );
                            } else {
                                // All tools done — fire ContinueTurn to re-enter the engine
                                let tx2 = tx.clone();
                                tokio::spawn(async move {
                                    let _ = tx2.send(AppEvent::ContinueTurn).await;
                                });
                            }
                        }
                        _ => {}
                    }
                }
                AppEvent::ContinueTurn => {
                    // All pending tool results have been fed back — run the next turn.
                    self.spinner.start(SpinnerMode::Thinking);

                    let (stream_tx, stream_rx) = mpsc::channel::<StreamEvent>(128);
                    let tx_forward = tx.clone();
                    tokio::spawn(async move {
                        let mut stream_rx = stream_rx;
                        while let Some(ev) = stream_rx.recv().await {
                            if tx_forward.send(AppEvent::Stream(ev)).await.is_err() {
                                break;
                            }
                        }
                    });

                    // Tell the engine task to run the next turn
                    let _ = engine_tx.send(EngineCommand::RunTurn(stream_tx)).await;
                }

                AppEvent::ShowAskUserDialog {
                    tool_use_id: _,
                    question,
                    options,
                } => {
                    // Show the AskUser input dialog; execution is paused in the tool
                    // via a oneshot channel until the user submits.
                    self.ask_user_dialog = Some(AskUserDialog::new(question, options));
                }

                AppEvent::AskUserResponse(answer) => {
                    // User submitted an answer — dismiss dialog and unblock the tool.
                    self.ask_user_dialog = None;
                    // Send the answer through the shared channel; the tool is waiting on it.
                    claude_tools::ask_user::send_user_answer(answer.clone());

                    // The tool will now return ToolResultData { answer } synchronously
                    // via its oneshot rx.  But the tool's future is already being awaited
                    // as part of the execute_tool call above.  After send_user_answer the
                    // pending execute_tool will complete and the engine/continuation will
                    // resume naturally — so we don't need to do anything extra here.
                    self.message_list.push(MessageEntry::System {
                        text: format!("Your answer: {}", answer),
                    });
                }

                AppEvent::TurnComplete(result) => {
                    match result {
                        Ok(TurnResult::Done(_stop_reason)) => {
                            self.spinner.stop();
                            self.engine_busy = false;
                        }
                        Ok(TurnResult::ToolUse(tool_uses)) => {
                            pending_tools = tool_uses
                                .into_iter()
                                .map(|info| PendingTool { info })
                                .collect();
                            pending_tool_index = 0;
                            self.spinner.stop();
                            self.check_next_tool_permission(
                                &pending_tools,
                                &mut pending_tool_index,
                                &perm_ctx,
                                &tools,
                                &tx,
                            );
                        }
                        Ok(TurnResult::ContinueRecovery) => {
                            self.message_list.push(MessageEntry::System {
                                text: "Continuing (max tokens recovery)...".to_string(),
                            });
                            let tx2 = tx.clone();
                            tokio::spawn(async move {
                                let _ = tx2.send(AppEvent::ContinueTurn).await;
                            });
                        }
                        Err(e) => {
                            self.spinner.stop();
                            self.engine_busy = false;
                            self.message_list.push(MessageEntry::System {
                                text: format!("Error: {}", e),
                            });
                        }
                    }
                }
            }
        }

        // Cleanup
        terminal::disable_raw_mode()?;
        execute!(
            io::stdout(),
            LeaveAlternateScreen,
            cursor::Show
        )?;
        Ok(())
    }

    /// Check permissions for the next tool in the pending list.
    /// If the tool is auto-allowed, execute it immediately and advance.
    /// If it needs user permission, show the dialog.
    fn check_next_tool_permission(
        &mut self,
        pending_tools: &[PendingTool],
        pending_tool_index: &mut usize,
        perm_ctx: &ToolPermissionContext,
        tools: &ToolRegistry,
        tx: &mpsc::Sender<AppEvent>,
    ) {
        while *pending_tool_index < pending_tools.len() {
            let tool = &pending_tools[*pending_tool_index];
            let info = &tool.info;
            *pending_tool_index += 1;

            // Determine if tool is read-only
            let is_read_only = tools
                .get(&info.name)
                .map(|t| t.is_read_only(&info.input))
                .unwrap_or(false);

            let decision = {
                use claude_core::permissions::evaluator::SimpleToolPermissions;
                let tool_perms = SimpleToolPermissions::new(&info.name, is_read_only);
                evaluate_permission(&tool_perms, &info.input, perm_ctx)
            };

            match decision {
                PermissionDecision::Allow(_) => {
                    // Auto-execute: we need to send an event to trigger execution
                    // For simplicity, send an "allow" permission response immediately
                    let tx2 = tx.clone();
                    tokio::spawn(async move {
                        let _ = tx2
                            .send(AppEvent::PermissionResponse("allow".to_string()))
                            .await;
                    });
                    return;
                }
                PermissionDecision::Ask(ask) => {
                    let input_preview = serde_json::to_string_pretty(&info.input)
                        .unwrap_or_else(|_| info.input.to_string());
                    self.permission_dialog = Some(PermissionDialog::new(
                        info.name.clone(),
                        ask.message,
                        input_preview,
                    ));
                    return;
                }
                PermissionDecision::Deny(deny) => {
                    let message = deny.message;
                    // Auto-deny, send deny response
                    let tx2 = tx.clone();
                    tokio::spawn(async move {
                        let _ = tx2
                            .send(AppEvent::PermissionResponse("deny".to_string()))
                            .await;
                    });
                    self.message_list.push(MessageEntry::System {
                        text: format!("Denied: {}", message),
                    });
                    return;
                }
            }
        }
    }

    fn handle_stream_event(&mut self, event: StreamEvent) {
        match event {
            StreamEvent::TextDelta { text } => {
                // Append to current assistant message, or create one
                if let Some(MessageEntry::Assistant { text: ref mut t }) =
                    self.message_list.messages_mut().last_mut()
                {
                    t.push_str(&text);
                } else {
                    self.message_list.push(MessageEntry::Assistant { text });
                }
            }
            StreamEvent::ThinkingDelta { text } => {
                if let Some(MessageEntry::Thinking { text: ref mut t }) =
                    self.message_list.messages_mut().last_mut()
                {
                    t.push_str(&text);
                } else {
                    self.message_list.push(MessageEntry::Thinking { text });
                }
            }
            StreamEvent::ToolStart {
                tool_use_id: _,
                name,
                input,
            } => {
                let summary = serde_json::to_string(&input).unwrap_or_else(|_| input.to_string());
                let summary = if summary.len() > 120 {
                    format!("{}...", &summary[..117])
                } else {
                    summary
                };
                self.message_list.push(MessageEntry::ToolUse {
                    name: name.clone(),
                    input_summary: summary,
                });
                self.spinner.start(SpinnerMode::Tool { name });
            }
            StreamEvent::ToolResult {
                tool_use_id: _,
                result,
            } => {
                self.message_list.push(MessageEntry::ToolResult {
                    name: "tool".to_string(),
                    output: truncate_result(
                        result.data.as_str().unwrap_or(&result.data.to_string()),
                    ),
                    is_error: result.is_error,
                });
            }
            StreamEvent::Done { stop_reason: _ } => {
                self.spinner.stop();
            }
            StreamEvent::UsageUpdate(ref usage) => {
                self.spinner.tokens = usage.output_tokens;
                self.total_tokens = self.total_tokens.saturating_add(usage.output_tokens);
                self.cost_tracker.add_usage(usage);
                // Sync shared state for slash commands
                if let Ok(mut state) = self.shared_state.lock() {
                    state.total_tokens = self.total_tokens;
                    state.request_count = self.cost_tracker.request_count();
                    state.total_cost_usd = self.cost_tracker.total_cost_usd();
                }
            }
            StreamEvent::RequestStart { request_id: _ } => {
                self.spinner.start(SpinnerMode::Thinking);
            }
            StreamEvent::Error(err) => {
                self.message_list.push(MessageEntry::System {
                    text: format!("Error: {}", err),
                });
            }
            _ => {}
        }
    }

    fn handle_key_standalone(&mut self, key: KeyEvent) {
        match (key.modifiers, key.code) {
            (KeyModifiers::CONTROL, KeyCode::Char('c')) => self.should_quit = true,
            (KeyModifiers::CONTROL, KeyCode::Char('d')) => self.should_quit = true,
            (KeyModifiers::CONTROL, KeyCode::Char('o')) => {
                self.message_list.toggle_thinking();
            }
            _ => {
                self.prompt.handle_key(key);
            }
        }
    }

    /// Handle mouse events (scroll wheel).
    fn handle_mouse(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::ScrollUp => {
                self.message_list.scroll_up(3);
            }
            MouseEventKind::ScrollDown => {
                self.message_list.scroll_down(3);
            }
            _ => {}
        }
    }

    /// Compute the context window usage percentage.
    fn context_percentage(&self) -> f64 {
        if self.context_window == 0 {
            return 0.0;
        }
        self.total_tokens as f64 / self.context_window as f64
    }

    /// Format a duration into a human-readable string (e.g. "2m 15s" or "1h 3m").
    #[allow(dead_code)]
    fn format_duration(dur: Duration) -> String {
        let secs = dur.as_secs();
        if secs < 60 {
            format!("{}s", secs)
        } else if secs < 3600 {
            format!("{}m {}s", secs / 60, secs % 60)
        } else {
            format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
        }
    }

    fn render(&mut self) -> Result<()> {
        let theme = &self.theme;
        let spinner = &self.spinner;
        let message_list = &self.message_list;
        let prompt = &self.prompt;
        let permission_dialog = &self.permission_dialog;
        let ask_user_dialog = &self.ask_user_dialog;
        let command_picker = &self.command_picker;
        let model_picker = &self.model_picker;
        let model_name = &self.model_name;
        let cost_header = self.cost_tracker.header_display();
        let context_pct = self.context_percentage();
        let engine_busy = self.engine_busy;

        // Read state for permission mode
        let perm_mode = self
            .shared_state
            .lock()
            .map(|s| s.permission_mode.clone())
            .unwrap_or_else(|_| "default".to_string());

        self.terminal.draw(|frame| {
            let area = frame.area();

            // Layout matching the original Claude Code:
            // - Messages area (scrollable, fills available space)
            // - Spinner row (1 line when active, 0 otherwise)
            // - Blank margin row (1 line, matches original marginTop=1 on prompt)
            // - Prompt input (3 lines: top border, input line, bottom border)
            let spinner_height = if spinner.active { 1 } else { 0 };
            let chunks = Layout::default()
                .constraints([
                    Constraint::Min(1),                 // Messages
                    Constraint::Length(spinner_height), // Spinner
                    Constraint::Length(3),              // Input (border + input + border)
                ])
                .split(area);

            // Messages area — pass theme for correct colors
            let msg_widget = MessageListWidget::new(message_list).theme(theme);
            frame.render_widget(msg_widget, chunks[0]);

            // Spinner (inline with messages, like the original)
            if spinner.active {
                frame.render_widget(spinner, chunks[1]);
            }

            // Prompt input with border matching the original's promptBorder color.
            // The original renders: borderStyle="round", borderColor=promptBorder,
            // borderLeft=false, borderRight=false, borderBottom=true.
            // The prompt border also carries status info (model, cost, context %).
            //
            // We render: ──model · cost (ctx%) · mode── as the top border text,
            // then ❯ input, then ───── as the bottom border.
            let input_area = chunks[2];
            let border_color = theme.prompt_border;

            {
                // Build the status line embedded in the top border
                let model_text = model_name.to_string();
                let cost_text = cost_header.clone();
                let pct_text = format!("{:.0}%", context_pct * 100.0);
                let mode_text = perm_mode.clone();

                // Status info: "model · cost (pct%) · mode"
                let status = format!(
                    "{} \u{00B7} {} ({}) \u{00B7} {}",
                    model_text, cost_text, pct_text, mode_text
                );

                // Build the top border with status text embedded:
                // ── status text ──────────
                let status_width = status.len();
                let remaining = (input_area.width as usize).saturating_sub(status_width + 4);
                let left_dashes = 2;
                let right_dashes = remaining;

                let token_color = if context_pct >= TOKEN_CRITICAL_THRESHOLD {
                    theme.error
                } else if context_pct >= TOKEN_WARNING_THRESHOLD {
                    theme.warning
                } else {
                    theme.inactive
                };

                let buf = frame.buffer_mut();

                if input_area.height >= 3 {
                    // Top border with status
                    let top_line = ratatui::text::Line::from(vec![
                        ratatui::text::Span::styled(
                            "\u{2500}".repeat(left_dashes),
                            ratatui::style::Style::default().fg(border_color),
                        ),
                        ratatui::text::Span::styled(
                            format!(" {} ", status),
                            ratatui::style::Style::default().fg(token_color),
                        ),
                        ratatui::text::Span::styled(
                            "\u{2500}".repeat(right_dashes),
                            ratatui::style::Style::default().fg(border_color),
                        ),
                    ]);
                    buf.set_line(input_area.x, input_area.y, &top_line, input_area.width);

                    // Prompt line: ❯ {text}
                    let prompt_style = if engine_busy {
                        ratatui::style::Style::default()
                            .fg(border_color)
                            .add_modifier(ratatui::style::Modifier::DIM)
                    } else {
                        ratatui::style::Style::default()
                    };

                    let prompt_line = ratatui::text::Line::from(vec![
                        ratatui::text::Span::styled("\u{276F} ", prompt_style),
                        ratatui::text::Span::raw(prompt.text().to_string()),
                    ]);
                    buf.set_line(
                        input_area.x,
                        input_area.y + 1,
                        &prompt_line,
                        input_area.width,
                    );

                    // Bottom border
                    let bottom_line = ratatui::text::Line::from(ratatui::text::Span::styled(
                        "\u{2500}".repeat(input_area.width as usize),
                        ratatui::style::Style::default().fg(border_color),
                    ));
                    buf.set_line(
                        input_area.x,
                        input_area.y + 2,
                        &bottom_line,
                        input_area.width,
                    );
                } else {
                    // Minimal: prompt line only
                    let prompt_style = if engine_busy {
                        ratatui::style::Style::default()
                            .fg(border_color)
                            .add_modifier(ratatui::style::Modifier::DIM)
                    } else {
                        ratatui::style::Style::default()
                    };
                    let prompt_line = ratatui::text::Line::from(vec![
                        ratatui::text::Span::styled("\u{276F} ", prompt_style),
                        ratatui::text::Span::raw(prompt.text().to_string()),
                    ]);
                    buf.set_line(input_area.x, input_area.y, &prompt_line, input_area.width);
                }
            }

            // Command picker overlay (positioned above the input area)
            if command_picker.visible && command_picker.has_entries() {
                // +2 for border top/bottom
                let picker_height = ((command_picker.filtered_count() as u16) + 2)
                    .max(3)
                    .min(area.height * 2 / 3);
                let picker_area = Rect::new(
                    area.x + 1,
                    chunks[2].y.saturating_sub(picker_height),
                    area.width.saturating_sub(2),
                    picker_height,
                );
                let picker_widget = CommandPickerWidget::new(command_picker);
                frame.render_widget(picker_widget, picker_area);
            }

            // Model picker — rendered inline above the prompt
            if model_picker.visible {
                let picker_height = model_picker.height().min(area.height.saturating_sub(4));
                let picker_area = Rect::new(
                    area.x,
                    chunks[2].y.saturating_sub(picker_height),
                    area.width,
                    picker_height,
                );
                let picker_widget = ModelPickerWidget::new(model_picker);
                frame.render_widget(picker_widget, picker_area);
            }

            // Permission dialog overlay
            if let Some(dialog) = permission_dialog {
                let dialog_area = centered_rect(60, 10, area);
                frame.render_widget(dialog, dialog_area);
            }

            // AskUser input dialog overlay
            if let Some(ref ask_dialog) = ask_user_dialog {
                let dialog_area = centered_rect(70, 10, area);
                frame.render_widget(ask_dialog, dialog_area);
            }

            // Show cursor at input position when no dialog is active
            if permission_dialog.is_none() && ask_user_dialog.is_none() && !engine_busy {
                // Cursor position: prompt char "❯ " is 2 display columns,
                // then the text up to cursor position
                let cursor_display_col = prompt.text()[..prompt.cursor()].chars().count() as u16;
                let cursor_x = input_area.x + 2 + cursor_display_col; // 2 = "❯ "
                let cursor_y = input_area.y + 1; // input is on row 1 of input_area
                if cursor_x < input_area.x + input_area.width && cursor_y < area.y + area.height {
                    frame.set_cursor_position((cursor_x, cursor_y));
                }
            }
        })?;

        // Update viewport height for page-up/down calculations
        self.viewport_height = self.terminal.size()?.height.saturating_sub(5);

        Ok(())
    }
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            LeaveAlternateScreen,
            cursor::Show
        );
    }
}

/// Execute a tool call.
async fn execute_tool(
    tools: &ToolRegistry,
    name: &str,
    input: &serde_json::Value,
    cwd: &std::path::Path,
    cancel: CancellationToken,
    read_file_state: std::sync::Arc<std::sync::Mutex<claude_tools::registry::ReadFileState>>,
) -> Result<ToolResultData> {
    let executor = tools
        .get(name)
        .ok_or_else(|| anyhow::anyhow!("Unknown tool: {}", name))?;
    let ctx = ToolUseContext {
        working_directory: cwd.to_path_buf(),
        read_file_state,
    };
    executor.call(input, &ctx, cancel, None).await
}

/// Truncate long tool results for display.
fn truncate_result(s: &str) -> String {
    const MAX_DISPLAY: usize = 2000;
    if s.len() <= MAX_DISPLAY {
        s.to_string()
    } else {
        format!("{}... ({} chars total)", &s[..MAX_DISPLAY], s.len())
    }
}

/// Calculate a centered rect within the given area.
fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let width = area.width * percent_x / 100;
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width, height.min(area.height))
}
