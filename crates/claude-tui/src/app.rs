use std::io::{self, Stdout};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event as CrosstermEvent, KeyCode, KeyEvent,
    KeyModifiers, MouseEvent, MouseEventKind,
};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{cursor, execute};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::widgets::Paragraph;
use ratatui::Terminal;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use claude_core::permissions::evaluator::evaluate_permission_sync;
use claude_core::permissions::types::{PermissionDecision, PermissionMode, ToolPermissionContext};
use claude_core::query::engine::{QueryEngine, ToolUseInfo, TurnResult};
use claude_core::types::events::{StreamEvent, ToolResultData};
use claude_tools::{ToolRegistry, ToolUseContext};

use claude_core::commands::builtin::build_default_commands;
use claude_core::commands::registry::{CommandContext, CommandRegistry, CommandResult, SharedCommandState};
use claude_core::cost::tracker::CostTracker;
use claude_core::plugins::skill;
use claude_core::plugins::types::Skill;

use std::sync::{Arc, Mutex};

use crate::theme::{detect_theme, Theme};
use crate::widgets::ask_user_dialog::AskUserDialog;
use crate::widgets::command_picker::{CommandPicker, CommandPickerEntry, CommandPickerWidget};
use crate::widgets::message_list::{MessageEntry, MessageList, MessageListWidget};
use crate::widgets::permission_dialog::PermissionDialog;
use crate::widgets::prompt_input::{InputAction, PromptInput, PromptInputWidget};
use crate::widgets::spinner::{SpinnerMode, SpinnerState};

/// Token budget warning thresholds.
const TOKEN_WARNING_THRESHOLD: f64 = 0.80; // Yellow warning at 80%
const TOKEN_CRITICAL_THRESHOLD: f64 = 0.95; // Red warning at 95%

pub enum AppEvent {
    Key(KeyEvent),
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
    /// Command registry for slash commands
    command_registry: CommandRegistry,
    /// Discovered skills
    skills: Vec<Skill>,
    /// Shared mutable state for slash commands (persistent across calls)
    shared_state: Arc<Mutex<SharedCommandState>>,
    /// Session start time for duration display
    session_start: std::time::Instant,
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
            command_registry,
            skills: Vec::new(),
            shared_state: Arc::new(Mutex::new(SharedCommandState::default())),
            session_start: std::time::Instant::now(),
            context_window: 200_000,
            viewport_height: 24,
        })
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
                working_directory: std::env::current_dir()
                    .unwrap_or_else(|_| PathBuf::from(".")),
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
                if state.dark_theme != (matches!(self.theme.fg, ratatui::style::Color::White)) {
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
        execute!(io::stdout(), EnterAlternateScreen, cursor::Hide)?;

        let (tx, mut rx) = mpsc::channel::<AppEvent>(100);

        // Spawn input reader
        let tx_input = tx.clone();
        tokio::spawn(async move {
            loop {
                if event::poll(Duration::from_millis(16)).unwrap_or(false) {
                    if let Ok(evt) = event::read() {
                        let app_evt = match evt {
                            CrosstermEvent::Key(k) => Some(AppEvent::Key(k)),
                            CrosstermEvent::Resize(w, h) => Some(AppEvent::Resize(w, h)),
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
                    AppEvent::Quit => self.should_quit = true,
                    _ => {}
                }
            }
        }

        terminal::disable_raw_mode()?;
        execute!(io::stdout(), LeaveAlternateScreen, cursor::Show)?;
        Ok(())
    }

    /// Run the TUI wired to the QueryEngine.
    pub async fn run_with_engine(
        &mut self,
        mut engine: QueryEngine,
        tools: ToolRegistry,
        cancel: CancellationToken,
        permission_mode: PermissionMode,
    ) -> Result<()> {
        terminal::enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, cursor::Hide)?;

        let (tx, mut rx) = mpsc::channel::<AppEvent>(256);

        // Spawn input reader
        let tx_input = tx.clone();
        tokio::spawn(async move {
            loop {
                if event::poll(Duration::from_millis(16)).unwrap_or(false) {
                    if let Ok(evt) = event::read() {
                        let app_evt = match evt {
                            CrosstermEvent::Key(k) => Some(AppEvent::Key(k)),
                            CrosstermEvent::Resize(w, h) => Some(AppEvent::Resize(w, h)),
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

        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

        // Shared read-file state for staleness tracking across tool calls
        let read_file_state = std::sync::Arc::new(std::sync::Mutex::new(
            claude_tools::registry::ReadFileState::new(),
        ));

        // Set permission mode in shared state
        {
            let mode_str = match &permission_mode {
                PermissionMode::Bypass => "bypass",
                PermissionMode::InteractiveOnly => "interactive-only",
                PermissionMode::Default => "default",
            };
            if let Ok(mut state) = self.shared_state.lock() {
                state.permission_mode = mode_str.to_string();
                // Detect initial theme from current app state
                state.dark_theme = matches!(self.theme.fg, ratatui::style::Color::White);
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
                            KeyCode::Char(c)
                                if k.modifiers.is_empty()
                                    || k.modifiers == KeyModifiers::SHIFT =>
                            {
                                self.prompt.handle_key(k);
                                let text = self.prompt.text().to_string();
                                let query = text.strip_prefix('/').unwrap_or("");
                                self.command_picker.set_query(query);
                                let _ = c;
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
                            _ => {}
                        }

                        // Route keys to prompt input
                        match self.prompt.handle_key(k) {
                            InputAction::Submit(text) => {
                                let _ = tx.send(AppEvent::SubmitPrompt(text)).await;
                            }
                            InputAction::None => {
                                // Check if user just typed `/` at start of input
                                if self.prompt.text() == "/" {
                                    let entries = self.build_picker_entries();
                                    self.command_picker.open(entries);
                                }
                            }
                        }
                    }
                }
                AppEvent::SubmitPrompt(text) => {
                    if self.engine_busy || text.trim().is_empty() {
                        continue;
                    }

                    // Try to handle as a slash command first
                    if text.starts_with('/') {
                        match self.try_execute_command(&text) {
                            Some(CommandAction::Display(output)) => {
                                // Action-type command: show output as system message
                                self.message_list.push(MessageEntry::System {
                                    text: output,
                                });

                                // Handle side effects from stateful commands
                                if let Ok(mut state) = self.shared_state.lock() {
                                    if state.clear_requested {
                                        state.clear_requested = false;
                                        engine.load_messages(Vec::new());
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
                                self.message_list.push(MessageEntry::User {
                                    text: text.clone(),
                                });
                                engine.add_user_message(&prompt_text);
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

                                match engine.run_turn(&stream_tx).await {
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
                                continue;
                            }
                            None => {
                                // Not a known command, treat as regular input
                            }
                        }
                    }

                    // Regular user message
                    // Add user message to display
                    self.message_list
                        .push(MessageEntry::User { text: text.clone() });

                    // Add to engine
                    engine.add_user_message(&text);
                    self.engine_busy = true;
                    self.spinner.start(SpinnerMode::Thinking);

                    // Run turn
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

                    match engine.run_turn(&stream_tx).await {
                        Ok(TurnResult::Done(_stop_reason)) => {
                            self.spinner.stop();
                            self.engine_busy = false;
                        }
                        Ok(TurnResult::ToolUse(tool_uses)) => {
                            // Start processing tool uses
                            pending_tools = tool_uses
                                .into_iter()
                                .map(|info| PendingTool { info })
                                .collect();
                            pending_tool_index = 0;
                            // Kick off permission check for first tool
                            self.check_next_tool_permission(
                                &pending_tools,
                                &mut pending_tool_index,
                                &perm_ctx,
                                &tools,
                                &tx,
                            );
                        }
                        Ok(TurnResult::ContinueRecovery) => {
                            // Re-run the turn for max_tokens recovery via ContinueTurn event
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
                            self.spinner
                                .start(SpinnerMode::Tool { name: info.name.clone() });
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
                                engine.add_tool_result(&info.id, &result_text, is_error);
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
                            engine.add_tool_result(
                                &info.id,
                                "Permission denied by user",
                                true,
                            );
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

                    match engine.run_turn(&stream_tx).await {
                        Ok(TurnResult::Done(_)) => {
                            self.spinner.stop();
                            self.engine_busy = false;
                        }
                        Ok(TurnResult::ContinueRecovery) => {
                            self.message_list.push(MessageEntry::System {
                                text: "Continuing (max tokens recovery)...".to_string(),
                            });
                            // Fire another ContinueTurn to keep going
                            let tx2 = tx.clone();
                            tokio::spawn(async move {
                                let _ = tx2.send(AppEvent::ContinueTurn).await;
                            });
                        }
                        Ok(TurnResult::ToolUse(tool_uses)) => {
                            // Another round of tool use — re-enter the permission/execute cycle
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
                        Err(e) => {
                            self.spinner.stop();
                            self.engine_busy = false;
                            self.message_list.push(MessageEntry::System {
                                text: format!("Error: {}", e),
                            });
                        }
                    }
                }

                AppEvent::ShowAskUserDialog { tool_use_id: _, question, options } => {
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
            }
        }

        // Cleanup
        terminal::disable_raw_mode()?;
        execute!(
            io::stdout(),
            DisableMouseCapture,
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

            let decision = evaluate_permission_sync(
                &info.name,
                &info.input,
                perm_ctx,
                is_read_only,
            );

            match decision {
                PermissionDecision::Allow => {
                    // Auto-execute: we need to send an event to trigger execution
                    // For simplicity, send an "allow" permission response immediately
                    let tx2 = tx.clone();
                    tokio::spawn(async move {
                        let _ = tx2.send(AppEvent::PermissionResponse("allow".to_string())).await;
                    });
                    return;
                }
                PermissionDecision::Ask { message } => {
                    let input_preview = serde_json::to_string_pretty(&info.input)
                        .unwrap_or_else(|_| info.input.to_string());
                    self.permission_dialog = Some(PermissionDialog::new(
                        info.name.clone(),
                        message,
                        input_preview,
                    ));
                    return;
                }
                PermissionDecision::Deny { message } => {
                    // Auto-deny, send deny response
                    let tx2 = tx.clone();
                    tokio::spawn(async move {
                        let _ = tx2.send(AppEvent::PermissionResponse("deny".to_string())).await;
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
                    self.message_list
                        .push(MessageEntry::Assistant { text });
                }
            }
            StreamEvent::ThinkingDelta { text } => {
                if let Some(MessageEntry::Thinking { text: ref mut t }) =
                    self.message_list.messages_mut().last_mut()
                {
                    t.push_str(&text);
                } else {
                    self.message_list
                        .push(MessageEntry::Thinking { text });
                }
            }
            StreamEvent::ToolStart {
                tool_use_id: _,
                name,
                input,
            } => {
                let summary = serde_json::to_string(&input)
                    .unwrap_or_else(|_| input.to_string());
                let summary = if summary.len() > 120 {
                    format!("{}...", &summary[..117])
                } else {
                    summary
                };
                self.message_list.push(MessageEntry::ToolUse {
                    name: name.clone(),
                    input_summary: summary,
                });
                self.spinner
                    .start(SpinnerMode::Tool { name });
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
        let model_name = &self.model_name;
        let cost_header = self.cost_tracker.header_display();
        let context_pct = self.context_percentage();
        let session_duration = self.session_start.elapsed();

        // Read state for permission mode, effort, fast/brief
        let (perm_mode, effort_level, fast_mode, brief_mode) = self
            .shared_state
            .lock()
            .map(|s| {
                (
                    s.permission_mode.clone(),
                    s.effort_level.clone(),
                    s.fast_mode,
                    s.brief_mode,
                )
            })
            .unwrap_or_else(|_| {
                ("default".to_string(), "medium".to_string(), false, false)
            });

        self.terminal.draw(|frame| {
            let area = frame.area();

            // Layout: header separator, header, separator, messages, spinner, separator, input
            let spinner_height = if spinner.active { 1 } else { 0 };
            let chunks = Layout::default()
                .constraints([
                    Constraint::Length(1), // Top border
                    Constraint::Length(1), // Header
                    Constraint::Length(1), // Header separator
                    Constraint::Min(1),   // Messages
                    Constraint::Length(spinner_height), // Spinner
                    Constraint::Length(3), // Input (with top border)
                ])
                .split(area);

            // Top border line
            let border_line = "─".repeat(area.width as usize);
            let top_border = Paragraph::new(border_line.clone())
                .style(ratatui::style::Style::default().fg(theme.border));
            frame.render_widget(top_border, chunks[0]);

            // Build header spans
            let mut header_spans: Vec<ratatui::text::Span> = vec![
                ratatui::text::Span::styled(
                    " Claude Code (Rust)",
                    ratatui::style::Style::default()
                        .fg(theme.accent)
                        .add_modifier(ratatui::style::Modifier::BOLD),
                ),
                ratatui::text::Span::styled(
                    " | ",
                    ratatui::style::Style::default().fg(theme.border),
                ),
                ratatui::text::Span::styled(
                    model_name.to_string(),
                    ratatui::style::Style::default().fg(theme.muted),
                ),
                ratatui::text::Span::styled(
                    " | ",
                    ratatui::style::Style::default().fg(theme.border),
                ),
            ];

            // Token count with context percentage and budget warning
            let pct_display = format!("{:.0}%", context_pct * 100.0);
            let token_color = if context_pct >= TOKEN_CRITICAL_THRESHOLD {
                theme.error
            } else if context_pct >= TOKEN_WARNING_THRESHOLD {
                theme.warning
            } else {
                theme.muted
            };
            header_spans.push(ratatui::text::Span::styled(
                format!("{} ({})", cost_header, pct_display),
                ratatui::style::Style::default().fg(token_color),
            ));

            // Token budget warning indicator
            if context_pct >= TOKEN_CRITICAL_THRESHOLD {
                header_spans.push(ratatui::text::Span::styled(
                    " CONTEXT FULL",
                    ratatui::style::Style::default()
                        .fg(theme.error)
                        .add_modifier(ratatui::style::Modifier::BOLD),
                ));
            } else if context_pct >= TOKEN_WARNING_THRESHOLD {
                header_spans.push(ratatui::text::Span::styled(
                    " CONTEXT HIGH",
                    ratatui::style::Style::default()
                        .fg(theme.warning)
                        .add_modifier(ratatui::style::Modifier::BOLD),
                ));
            }

            // Permission mode
            header_spans.push(ratatui::text::Span::styled(
                " | ",
                ratatui::style::Style::default().fg(theme.border),
            ));
            header_spans.push(ratatui::text::Span::styled(
                perm_mode,
                ratatui::style::Style::default().fg(theme.muted),
            ));

            // Mode indicators (fast/brief/effort)
            let mut indicators = Vec::new();
            if fast_mode {
                indicators.push("fast");
            }
            if brief_mode {
                indicators.push("brief");
            }
            if effort_level != "medium" {
                // effort_level is on the stack; we need to handle this
                // We'll format it into the indicators below
            }
            // Build the indicator suffix
            let effort_indicator = if effort_level != "medium" {
                format!("effort:{}", effort_level)
            } else {
                String::new()
            };

            if !indicators.is_empty() || !effort_indicator.is_empty() {
                header_spans.push(ratatui::text::Span::styled(
                    " | ",
                    ratatui::style::Style::default().fg(theme.border),
                ));
                let mut parts: Vec<String> =
                    indicators.iter().map(|s| s.to_string()).collect();
                if !effort_indicator.is_empty() {
                    parts.push(effort_indicator);
                }
                header_spans.push(ratatui::text::Span::styled(
                    parts.join(" "),
                    ratatui::style::Style::default().fg(ratatui::style::Color::Cyan),
                ));
            }

            // Session duration
            header_spans.push(ratatui::text::Span::styled(
                " | ",
                ratatui::style::Style::default().fg(theme.border),
            ));
            header_spans.push(ratatui::text::Span::styled(
                Self::format_duration(session_duration),
                ratatui::style::Style::default().fg(theme.muted),
            ));

            let header = Paragraph::new(ratatui::text::Line::from(header_spans));
            frame.render_widget(header, chunks[1]);

            // Header separator
            let header_sep = Paragraph::new(border_line)
                .style(ratatui::style::Style::default().fg(theme.border));
            frame.render_widget(header_sep, chunks[2]);

            // Messages area
            let msg_widget = MessageListWidget::new(message_list);
            frame.render_widget(msg_widget, chunks[3]);

            // Spinner
            if spinner.active {
                frame.render_widget(spinner, chunks[4]);
            }

            // Input
            let input_widget = PromptInputWidget::new(prompt);
            frame.render_widget(input_widget, chunks[5]);

            // Command picker overlay (positioned above the input area)
            if command_picker.visible {
                let picker_height = 12u16.min(area.height / 3);
                let picker_area = Rect::new(
                    area.x + 1,
                    chunks[5].y.saturating_sub(picker_height),
                    area.width.saturating_sub(2),
                    picker_height,
                );
                let picker_widget = CommandPickerWidget::new(command_picker);
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
        })?;

        // Update viewport height for page-up/down calculations
        self.viewport_height = self.terminal.size()?.height.saturating_sub(7);

        Ok(())
    }
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            DisableMouseCapture,
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
