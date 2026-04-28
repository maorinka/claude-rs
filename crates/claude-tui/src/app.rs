use std::io::{self, Stdout};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{
    self, Event as CrosstermEvent, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{cursor, execute};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Terminal;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use claude_core::hooks::{
    get_global_runner, resolve_hook_permission_decision, run_post_tool_use_failure_hooks,
    run_post_tool_use_hooks, run_pre_tool_use_hooks, set_global_runner, HookRunner,
    ResolvedPermission,
};
use claude_core::permissions::evaluator::{
    apply_permission_updates, evaluate_permission, persist_permission_updates,
    sync_permission_rules_from_disk,
};
use claude_core::permissions::types::{
    PermissionBehavior, PermissionDecision, PermissionDecisionReason, PermissionMode,
    PermissionRuleSource, PermissionRuleValue, PermissionUpdate, ToolPermissionContext,
};
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

use crate::theme::{detect_theme, resolve_theme, Theme, ThemeSetting};
use crate::widgets::ask_user_dialog::AskUserDialog;
use crate::widgets::command_picker::{CommandPicker, CommandPickerEntry, CommandPickerWidget};
use crate::widgets::message_list::{MessageEntry, MessageList, MessageListWidget};
use crate::widgets::model_picker::{ModelPicker, ModelPickerWidget};
use crate::widgets::permission_dialog::PermissionDialog;
use crate::widgets::prompt_input::{InputAction, PromptInput};
use crate::widgets::spinner::{SpinnerMode, SpinnerState};
use crate::widgets::theme_picker::{ThemePicker, ThemePickerWidget};

/// Token budget warning thresholds.
const TOKEN_WARNING_THRESHOLD: f64 = 0.80; // Yellow warning at 80%
const TOKEN_CRITICAL_THRESHOLD: f64 = 0.95; // Red warning at 95%
const DOUBLE_PRESS_TIMEOUT_MS: u64 = 800;

fn permission_mode_hook_name(mode: &PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Default => "default",
        PermissionMode::AcceptEdits => "acceptEdits",
        PermissionMode::Auto => "auto",
        PermissionMode::Plan => "plan",
        PermissionMode::BypassPermissions => "bypassPermissions",
        PermissionMode::DontAsk => "dontAsk",
        PermissionMode::Bubble => "bubble",
    }
}

fn merge_hook_updated_input(
    input: &serde_json::Value,
    updated: &Option<std::collections::HashMap<String, serde_json::Value>>,
) -> serde_json::Value {
    let Some(updated) = updated else {
        return input.clone();
    };
    let mut merged = input.clone();
    if let Some(obj) = merged.as_object_mut() {
        for (key, value) in updated {
            obj.insert(key.clone(), value.clone());
        }
    }
    merged
}

fn permission_decision_to_rule_check(
    decision: &PermissionDecision,
) -> claude_core::hooks::RuleCheckResult {
    match decision {
        PermissionDecision::Allow(_) => claude_core::hooks::RuleCheckResult::NoMatch,
        PermissionDecision::Ask(_) => claude_core::hooks::RuleCheckResult::Ask,
        PermissionDecision::Deny(deny) => {
            claude_core::hooks::RuleCheckResult::Deny(Some(deny.message.clone()))
        }
    }
}

fn hook_blocking_errors_text(errors: &[claude_core::hooks::HookBlockingError]) -> Option<String> {
    if errors.is_empty() {
        None
    } else {
        Some(
            errors
                .iter()
                .map(|err| err.blocking_error.clone())
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RewindRestoreOption {
    RestoreConversation,
    SummarizeFromHere,
    NeverMind,
}

impl RewindRestoreOption {
    fn label(self) -> &'static str {
        match self {
            Self::RestoreConversation => "Restore conversation",
            Self::SummarizeFromHere => "Summarize from here",
            Self::NeverMind => "Never mind",
        }
    }
}

#[derive(Debug, Clone)]
struct RewindConfirmation {
    message_index: usize,
    prompt_text: String,
    selected: usize,
}

impl RewindConfirmation {
    fn selected_option(&self) -> RewindRestoreOption {
        match self.selected {
            0 => RewindRestoreOption::RestoreConversation,
            1 => RewindRestoreOption::SummarizeFromHere,
            _ => RewindRestoreOption::NeverMind,
        }
    }

    fn next(&mut self) {
        self.selected = (self.selected + 1) % 3;
    }

    fn prev(&mut self) {
        self.selected = self.selected.checked_sub(1).unwrap_or(2);
    }
}

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
    /// Fired when a background tool execution completes.
    ToolExecutionComplete {
        tool_idx: usize,
        result: Result<ToolResultData, String>,
    },
    /// Fired when the background Haiku permission-explainer call finishes.
    /// `Some(text)` populates the dialog's explanation field; `None`
    /// silently leaves it unset (no model registered, or the call failed).
    PermissionExplanation(Option<String>),
}

/// Commands sent to the dedicated engine task via a channel.
/// The engine owns the `QueryEngine` exclusively; all interaction goes through
/// these non-blocking sends so the event loop never awaits the engine.
enum EngineCommand {
    AddUserMessage(String),
    AddUserContext(String),
    AddUserContextMessage(String),
    AddToolResult {
        id: String,
        content: String,
        is_error: bool,
    },
    RunTurn(mpsc::Sender<StreamEvent>),
    LoadMessages(Vec<serde_json::Value>),
    PartialCompactFrom {
        messages: Vec<serde_json::Value>,
        pivot_index: usize,
        response:
            oneshot::Sender<anyhow::Result<claude_core::compact::compactor::PartialCompactResult>>,
    },
    SetModel(String),
    /// Replace the cancellation token after an Escape-cancel so the next turn
    /// can be independently cancelled.
    SetCancelToken(CancellationToken),
}

/// Pending tool that needs permission before execution.
struct PendingTool {
    info: ToolUseInfo,
    permission_updates_on_allow: Vec<PermissionUpdate>,
}

fn skills_reminder_block(skills: &[Skill]) -> String {
    let mut skills_text = String::from(
        "<system-reminder>\nThe following skills are available for use with the Skill tool:\n\n",
    );
    for skill in skills {
        skills_text.push_str(&format!("- {}: {}", skill.name, skill.description));
        if let Some(ref hint) = skill.when_to_use {
            skills_text.push_str(&format!(" (use when: {})", hint));
        }
        skills_text.push('\n');
    }
    skills_text.push_str("</system-reminder>\n");
    skills_text
}

fn dynamic_skill_file_paths(tool_name: &str, input: &serde_json::Value) -> Vec<PathBuf> {
    let key = match tool_name {
        "Read" | "Edit" | "Write" => "file_path",
        _ => return Vec::new(),
    };
    input
        .get(key)
        .and_then(|value| value.as_str())
        .map(PathBuf::from)
        .into_iter()
        .collect()
}

fn fallback_allow_update_for_tool(tool_name: &str) -> PermissionUpdate {
    PermissionUpdate::AddRules {
        destination: PermissionRuleSource::LocalSettings,
        rules: vec![PermissionRuleValue {
            tool_name: tool_name.to_string(),
            rule_content: None,
        }],
        behavior: PermissionBehavior::Allow,
    }
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
    /// Messages queued while engine was busy — sent FIFO after turns complete.
    /// Matches TS commandQueue (messageQueueManager.ts).
    message_queue: Vec<String>,
    /// Model name for display in the header
    model_name: String,
    /// Running total of tokens used in this session
    total_tokens: u64,
    /// Cost tracker for the session -- accumulates usage from StreamEvent::UsageUpdate
    cost_tracker: CostTracker,
    /// Command picker overlay (shown on `/` at start of input)
    command_picker: CommandPicker,
    /// Rewind picker overlay (shown on Escape while idle)
    rewind_picker: CommandPicker,
    /// Confirm restore/summarize after choosing a rewind checkpoint.
    rewind_confirmation: Option<RewindConfirmation>,
    /// Last idle Escape press, used to match TS double-press behavior.
    last_idle_escape: Option<Instant>,
    /// Model picker overlay (shown on `/model`)
    model_picker: ModelPicker,
    /// Theme picker overlay (shown on `/theme`)
    theme_picker: ThemePicker,
    /// Current theme setting (auto, dark, light, etc.)
    theme_setting: ThemeSetting,
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
            message_queue: Vec::new(),
            model_name: "claude-sonnet-4-6".to_string(),
            total_tokens: 0,
            cost_tracker: CostTracker::new("claude-sonnet-4-6"),
            command_picker: CommandPicker::new(),
            rewind_picker: CommandPicker::new(),
            rewind_confirmation: None,
            last_idle_escape: None,
            model_picker: ModelPicker::new(),
            theme_picker: ThemePicker::new(),
            theme_setting: ThemeSetting::Auto,
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

    fn enqueue_message(&mut self, text: String) {
        self.message_queue.push(text);
        self.prompt.clear();
        self.spinner.queued_count = 0;
    }

    fn pop_queued_message(&mut self) -> Option<String> {
        if self.message_queue.is_empty() {
            return None;
        }
        self.spinner.queued_count = 0;
        Some(self.message_queue.remove(0))
    }

    fn edit_last_queued_message(&mut self) -> bool {
        let Some(text) = self.message_queue.pop() else {
            return false;
        };
        self.spinner.queued_count = 0;
        self.prompt.set_text(text);
        true
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
                display_name: None,
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
                    display_name: None,
                });
            }
        }

        entries
    }

    fn build_rewind_entries(&self) -> Vec<CommandPickerEntry> {
        self.message_list
            .messages()
            .iter()
            .enumerate()
            .rev()
            .filter_map(|(idx, msg)| {
                let MessageEntry::User { text } = msg else {
                    return None;
                };
                Some(CommandPickerEntry {
                    name: format!("#{}", idx),
                    description: "No code changes".to_string(),
                    display_name: Some(truncate_chars(text.replace('\n', " ").trim(), 96)),
                })
            })
            .collect()
    }

    fn restore_rewind(&mut self, idx: usize) -> Option<String> {
        let prompt_text = match self.message_list.messages().get(idx) {
            Some(MessageEntry::User { text }) => text.clone(),
            _ => return None,
        };
        self.message_list.truncate(idx);
        Some(prompt_text)
    }

    fn prepare_rewind_confirmation(&self, idx: usize) -> Option<RewindConfirmation> {
        let prompt_text = match self.message_list.messages().get(idx) {
            Some(MessageEntry::User { text }) => text.clone(),
            _ => return None,
        };
        Some(RewindConfirmation {
            message_index: idx,
            prompt_text,
            selected: 0,
        })
    }

    #[cfg(test)]
    fn summarize_rewind_from(&mut self, idx: usize) -> Option<String> {
        let prompt_text = match self.message_list.messages().get(idx) {
            Some(MessageEntry::User { text }) => text.clone(),
            _ => return None,
        };
        let removed = self.message_list.messages()[idx..]
            .iter()
            .filter_map(|entry| match entry {
                MessageEntry::User { text } => Some(format!("User: {text}")),
                MessageEntry::Assistant { text } => Some(format!("Assistant: {text}")),
                MessageEntry::ToolUse {
                    name,
                    input_summary,
                    ..
                } => Some(format!("Tool use {name}: {input_summary}")),
                MessageEntry::ToolResult {
                    name,
                    output,
                    is_error,
                    ..
                } => {
                    let status = if *is_error { "error" } else { "result" };
                    Some(format!(
                        "Tool {name} {status}: {}",
                        truncate_chars(output, 1000)
                    ))
                }
                MessageEntry::Thinking { text } => Some(format!("Thinking: {text}")),
                MessageEntry::System { text } => Some(format!("System: {text}")),
                MessageEntry::CompactionSummary { text } => {
                    Some(format!("Compaction summary: {text}"))
                }
                MessageEntry::Logo { .. } => None,
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        self.message_list.truncate(idx);
        let summary = if removed.trim().is_empty() {
            "No messages were available to summarize after this point.".to_string()
        } else {
            format!("Messages after this point were summarized locally:\n\n{removed}")
        };
        let compact_user_msg =
            claude_core::compact::prompt::format_compact_user_message_simple(&summary);
        self.message_list.push(MessageEntry::CompactionSummary {
            text: compact_user_msg,
        });
        Some(prompt_text)
    }

    fn prepare_partial_compact_from(
        &self,
        idx: usize,
    ) -> Option<(String, Vec<serde_json::Value>, usize)> {
        let prompt_text = match self.message_list.messages().get(idx) {
            Some(MessageEntry::User { text }) => text.clone(),
            _ => return None,
        };
        let messages = reconstruct_engine_messages(self.message_list.messages());
        let pivot_index = reconstruct_engine_messages(&self.message_list.messages()[..idx]).len();
        Some((prompt_text, messages, pivot_index))
    }

    fn apply_partial_compact_result(
        &mut self,
        idx: usize,
        compacted: &[serde_json::Value],
    ) -> Option<String> {
        let prompt_text = match self.message_list.messages().get(idx) {
            Some(MessageEntry::User { text }) => text.clone(),
            _ => return None,
        };
        let summary_text = compacted.last().and_then(message_text).unwrap_or_else(|| {
            "Conversation summary was generated, but no summary text was returned.".to_string()
        });
        self.message_list.truncate(idx);
        self.message_list
            .push(MessageEntry::CompactionSummary { text: summary_text });
        Some(prompt_text)
    }

    fn open_rewind_on_double_escape(&mut self) {
        if !self.prompt.is_empty() {
            self.last_idle_escape = None;
            return;
        }

        let now = Instant::now();
        let is_double_press = self
            .last_idle_escape
            .map(|last| now.duration_since(last) <= Duration::from_millis(DOUBLE_PRESS_TIMEOUT_MS))
            .unwrap_or(false);

        if is_double_press {
            self.last_idle_escape = None;
            self.rewind_confirmation = None;
            let entries = self.build_rewind_entries();
            if !entries.is_empty() {
                self.rewind_picker.open(entries);
            }
        } else {
            self.last_idle_escape = Some(now);
        }
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
                // Theme change: check if shared state has a new theme_setting
                if let Ok(new_setting) = state.theme_setting.parse::<ThemeSetting>() {
                    if new_setting != self.theme_setting {
                        self.theme_setting = new_setting;
                        self.theme = resolve_theme(new_setting);
                    }
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
        execute!(io::stdout(), LeaveAlternateScreen, cursor::Show)?;
        Ok(())
    }

    /// Run the TUI wired to the QueryEngine.
    pub async fn run_with_engine(
        &mut self,
        engine: QueryEngine,
        tools: ToolRegistry,
        mut cancel: CancellationToken,
        initial_permission_context: ToolPermissionContext,
        api_session_id: String,
    ) -> Result<()> {
        terminal::enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, cursor::Hide)?;

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
                        EngineCommand::AddUserContext(text) => {
                            engine.append_user_context_block(text);
                        }
                        EngineCommand::AddUserContextMessage(text) => {
                            engine.add_user_context_message(text);
                        }
                        EngineCommand::AddToolResult {
                            id,
                            content,
                            is_error,
                        } => {
                            engine.add_tool_result(&id, &content, is_error);
                        }
                        EngineCommand::RunTurn(stream_tx) => {
                            let result = engine.run_turn(&stream_tx).await;
                            let _ = app_tx.send(AppEvent::TurnComplete(result)).await;
                        }
                        EngineCommand::LoadMessages(msgs) => {
                            engine.load_messages(msgs);
                        }
                        EngineCommand::PartialCompactFrom {
                            messages,
                            pivot_index,
                            response,
                        } => {
                            engine.load_messages(messages);
                            let result = engine.partial_compact_from(pivot_index).await;
                            let _ = response.send(result);
                        }
                        EngineCommand::SetModel(model) => {
                            engine.set_model(model);
                        }
                        EngineCommand::SetCancelToken(token) => {
                            engine.set_cancel_token(token);
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
        let mut perm_ctx = initial_permission_context;
        let mut settings_fingerprint = claude_core::permissions::settings_change_fingerprint(&cwd);
        let mut last_settings_check = Instant::now();
        let permission_mode = perm_ctx.mode.clone();
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
                // Sync initial theme setting
                state.theme_setting = self.theme_setting.to_string();
                state.dark_theme = matches!(
                    self.theme_setting,
                    ThemeSetting::Auto
                        | ThemeSetting::Named(
                            crate::theme::ThemeName::Dark
                                | crate::theme::ThemeName::DarkDaltonized
                                | crate::theme::ThemeName::DarkAnsi
                        )
                );
            }
        }

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
                    if last_settings_check.elapsed() >= Duration::from_millis(500) {
                        last_settings_check = Instant::now();
                        let next_fingerprint =
                            claude_core::permissions::settings_change_fingerprint(&cwd);
                        if next_fingerprint != settings_fingerprint {
                            settings_fingerprint = next_fingerprint;
                            let updated_rules =
                                claude_core::permissions::load_permission_rules_from_disk_by_source(
                                    &cwd,
                                );
                            perm_ctx = sync_permission_rules_from_disk(perm_ctx, &updated_rules);
                            let settings_value =
                                claude_core::permissions::load_raw_settings_value_with_plugin_hooks(
                                    &cwd,
                                );
                            set_global_runner(Arc::new(HookRunner::from_settings(
                                &settings_value,
                                cwd.display().to_string(),
                                api_session_id.clone(),
                                String::new(),
                            )));
                            if let Ok(mut state) = self.shared_state.lock() {
                                state.permission_mode =
                                    permission_mode_hook_name(&perm_ctx.mode).to_string();
                            }
                        }
                    }

                    // Reactive queue drain — mirrors TS useQueueProcessor:
                    // when engine becomes idle and there's a queued message, dispatch it.
                    if !self.engine_busy {
                        if let Some(queued) = self.pop_queued_message() {
                            let tx2 = tx.clone();
                            tokio::spawn(async move {
                                let _ = tx2.send(AppEvent::SubmitPrompt(queued)).await;
                            });
                        }
                    }
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
                            KeyCode::Down | KeyCode::Right => {
                                if let Some(ref mut dialog) = self.permission_dialog {
                                    dialog.next_button();
                                }
                            }
                            KeyCode::Up | KeyCode::Left => {
                                if let Some(ref mut dialog) = self.permission_dialog {
                                    dialog.prev_button();
                                }
                            }
                            KeyCode::Esc => {
                                let _ = tx
                                    .send(AppEvent::PermissionResponse("deny".to_string()))
                                    .await;
                            }
                            KeyCode::Char('e') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                                if let Some(ref mut dialog) = self.permission_dialog {
                                    dialog.toggle_explanation();
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
                                    let display =
                                        claude_core::commands::builtin::render_model_name(&model);
                                    self.model_name = model.clone();
                                    let _ = engine_tx
                                        .send(EngineCommand::SetModel(model.clone()))
                                        .await;
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
                    } else if self.theme_picker.visible {
                        // Route keys to theme picker: ↑↓ navigate, Enter confirm, Esc cancel
                        match k.code {
                            KeyCode::Esc => {
                                self.theme_picker.close();
                            }
                            KeyCode::Up => self.theme_picker.prev(),
                            KeyCode::Down => self.theme_picker.next(),
                            KeyCode::Enter => {
                                if let Some(setting) = self.theme_picker.confirm() {
                                    self.theme_setting = setting;
                                    self.theme = resolve_theme(setting);
                                    if let Ok(mut state) = self.shared_state.lock() {
                                        state.theme_setting = setting.to_string();
                                        // Keep dark_theme in sync for legacy consumers
                                        state.dark_theme = matches!(
                                            setting,
                                            ThemeSetting::Auto
                                                | ThemeSetting::Named(
                                                    crate::theme::ThemeName::Dark
                                                        | crate::theme::ThemeName::DarkDaltonized
                                                        | crate::theme::ThemeName::DarkAnsi
                                                )
                                        );
                                    }
                                    self.message_list.push(MessageEntry::System {
                                        text: format!("Theme set to {}", setting),
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
                            KeyCode::BackTab | KeyCode::Left => {
                                self.command_picker.prev();
                            }
                            KeyCode::Down | KeyCode::Tab | KeyCode::Right => {
                                self.command_picker.next();
                            }
                            KeyCode::Enter => {
                                if let Some(name) = self.command_picker.selected_name() {
                                    let cmd_text = format!("/{}", name);
                                    self.command_picker.close();
                                    self.prompt.clear();
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
                                    if query.chars().any(char::is_whitespace) {
                                        self.command_picker.close();
                                    } else {
                                        self.command_picker.set_query(query);
                                    }
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
                                    if q.chars().any(char::is_whitespace) {
                                        self.command_picker.close();
                                    } else {
                                        self.command_picker.set_query(q);
                                    }
                                } else {
                                    self.command_picker.close();
                                }
                            }
                            _ => {}
                        }
                    } else if self.rewind_confirmation.is_some() {
                        match k.code {
                            KeyCode::Esc => {
                                self.rewind_confirmation = None;
                                self.rewind_picker.close();
                                continue;
                            }
                            KeyCode::Up => {
                                if let Some(confirm) = &mut self.rewind_confirmation {
                                    confirm.prev();
                                }
                                continue;
                            }
                            KeyCode::Down => {
                                if let Some(confirm) = &mut self.rewind_confirmation {
                                    confirm.next();
                                }
                                continue;
                            }
                            KeyCode::Char('1') => {
                                if let Some(confirm) = &mut self.rewind_confirmation {
                                    confirm.selected = 0;
                                }
                                continue;
                            }
                            KeyCode::Char('2') => {
                                if let Some(confirm) = &mut self.rewind_confirmation {
                                    confirm.selected = 1;
                                }
                                continue;
                            }
                            KeyCode::Char('3') => {
                                if let Some(confirm) = &mut self.rewind_confirmation {
                                    confirm.selected = 2;
                                }
                                continue;
                            }
                            KeyCode::Enter => {
                                let Some(confirm) = self.rewind_confirmation.take() else {
                                    continue;
                                };
                                match confirm.selected_option() {
                                    RewindRestoreOption::RestoreConversation => {
                                        if let Some(prompt_text) =
                                            self.restore_rewind(confirm.message_index)
                                        {
                                            let messages = reconstruct_engine_messages(
                                                self.message_list.messages(),
                                            );
                                            let _ = engine_tx
                                                .send(EngineCommand::LoadMessages(messages))
                                                .await;
                                            self.prompt.set_text(prompt_text);
                                            self.message_list.push(MessageEntry::System {
                                                text: "Rewound conversation. Edit the restored prompt, then press Enter to resubmit.".to_string(),
                                            });
                                        }
                                    }
                                    RewindRestoreOption::SummarizeFromHere => {
                                        if let Some((_, messages, pivot_index)) =
                                            self.prepare_partial_compact_from(confirm.message_index)
                                        {
                                            self.message_list.push(MessageEntry::System {
                                                text: "Summarizing conversation...".to_string(),
                                            });
                                            let (response_tx, response_rx) = oneshot::channel();
                                            let _ = engine_tx
                                                .send(EngineCommand::PartialCompactFrom {
                                                    messages,
                                                    pivot_index,
                                                    response: response_tx,
                                                })
                                                .await;
                                            match response_rx.await {
                                                Ok(Ok(compacted)) => {
                                                    if let Some(prompt_text) = self
                                                        .apply_partial_compact_result(
                                                            confirm.message_index,
                                                            &compacted.messages,
                                                        )
                                                    {
                                                        self.prompt.set_text(prompt_text);
                                                        for message in compacted.hook_messages {
                                                            self.message_list.push(
                                                                MessageEntry::System {
                                                                    text: message,
                                                                },
                                                            );
                                                        }
                                                        self.message_list.push(MessageEntry::System {
                                                            text: "Conversation summarized. Edit the restored prompt, then press Enter to resubmit.".to_string(),
                                                        });
                                                    }
                                                }
                                                Ok(Err(err)) => {
                                                    self.message_list.push(MessageEntry::System {
                                                        text: format!(
                                                            "Unable to summarize conversation: {err}"
                                                        ),
                                                    });
                                                }
                                                Err(_) => {
                                                    self.message_list.push(MessageEntry::System {
                                                        text: "Unable to summarize conversation: engine stopped responding.".to_string(),
                                                    });
                                                }
                                            }
                                        }
                                    }
                                    RewindRestoreOption::NeverMind => {}
                                }
                                self.rewind_picker.close();
                                continue;
                            }
                            _ => {}
                        }
                    } else if self.rewind_picker.visible {
                        match k.code {
                            KeyCode::Esc => {
                                self.rewind_picker.close();
                                continue;
                            }
                            KeyCode::Up => {
                                self.rewind_picker.prev();
                                continue;
                            }
                            KeyCode::Down => {
                                self.rewind_picker.next();
                                continue;
                            }
                            KeyCode::Enter => {
                                if let Some(name) = self.rewind_picker.selected_name() {
                                    if let Some(idx) = parse_rewind_picker_name(name) {
                                        self.rewind_confirmation =
                                            self.prepare_rewind_confirmation(idx);
                                    }
                                }
                                continue;
                            }
                            _ => {}
                        }
                    } else {
                        // Escape: cancel in-progress response (does not quit).
                        if k.code == KeyCode::Esc && k.modifiers.is_empty() {
                            if self.engine_busy {
                                cancel.cancel();
                                // Preserve any partial streaming text already in the
                                // message list (appended incrementally by TextDelta handlers).
                                self.engine_busy = false;
                                self.spinner.stop();
                                self.spinner.queued_count = 0;
                                self.message_list.clear_running_tools();
                                self.message_list.push(MessageEntry::System {
                                    text: "[Request interrupted by user]".to_string(),
                                });

                                // Dispatch queued message if any
                                if let Some(queued) = self.pop_queued_message() {
                                    let tx2 = tx.clone();
                                    tokio::spawn(async move {
                                        let _ = tx2.send(AppEvent::SubmitPrompt(queued)).await;
                                    });
                                }

                                // Add error tool_results for any pending tools so the
                                // message history stays valid for the API.
                                for pt in &pending_tools {
                                    let _ = engine_tx
                                        .send(EngineCommand::AddToolResult {
                                            id: pt.info.id.clone(),
                                            content: "Interrupted by user".to_string(),
                                            is_error: true,
                                        })
                                        .await;
                                }
                                pending_tools.clear();
                                pending_tool_index = 0;

                                // Replace the exhausted token so the next turn can be cancelled.
                                cancel = CancellationToken::new();
                                let _ = engine_tx
                                    .send(EngineCommand::SetCancelToken(cancel.clone()))
                                    .await;
                            } else {
                                self.open_rewind_on_double_escape();
                            }
                            continue;
                        }

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
                            (KeyModifiers::NONE, KeyCode::Up)
                                if self.engine_busy
                                    && self.prompt.is_empty()
                                    && !self.message_queue.is_empty() =>
                            {
                                self.edit_last_queued_message();
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
                                if let Some(query) = text.strip_prefix('/') {
                                    if !self.command_picker.visible {
                                        // Open picker on first `/`
                                        let entries = self.build_picker_entries();
                                        self.command_picker.open(entries);
                                    }
                                    // Update filter with text after `/`
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
                    if text.trim().is_empty() {
                        continue;
                    }
                    if self.engine_busy {
                        // TS behavior (handlePromptSubmit.ts:336): enqueue and clear
                        // input. The render path keeps the queued text visible
                        // above the prompt and lets Up move it back into the
                        // editor.
                        self.enqueue_message(text);
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
                        let new_model =
                            claude_core::commands::builtin::parse_user_specified_model(args);
                        let display = claude_core::commands::builtin::render_model_name(&new_model);
                        self.model_name = new_model.clone();
                        let _ = engine_tx
                            .send(EngineCommand::SetModel(new_model.clone()))
                            .await;
                        if let Some(shared) = Some(&self.shared_state) {
                            if let Ok(mut state) = shared.lock() {
                                state.model = new_model;
                            }
                        }
                        self.message_list.push(MessageEntry::System {
                            text: format!("Set model to {}", display),
                        });
                        continue;
                    }

                    // Intercept /theme to open interactive picker
                    if text.trim() == "/theme" || text.trim().starts_with("/theme ") {
                        let args = text.trim().strip_prefix("/theme").unwrap_or("").trim();
                        if args.is_empty() {
                            // No args: open interactive picker
                            self.prompt.clear();
                            self.theme_picker.open(self.theme_setting);
                            continue;
                        }
                        // Has args: set theme directly via ThemeSetting
                        if let Ok(setting) = args.parse::<ThemeSetting>() {
                            self.theme_setting = setting;
                            self.theme = resolve_theme(setting);
                            if let Ok(mut state) = self.shared_state.lock() {
                                state.theme_setting = setting.to_string();
                                state.dark_theme = matches!(
                                    setting,
                                    ThemeSetting::Auto
                                        | ThemeSetting::Named(
                                            crate::theme::ThemeName::Dark
                                                | crate::theme::ThemeName::DarkDaltonized
                                                | crate::theme::ThemeName::DarkAnsi
                                        )
                                );
                            }
                            self.message_list.push(MessageEntry::System {
                                text: format!("Theme set to {}", setting),
                            });
                        } else {
                            self.message_list.push(MessageEntry::System {
                                text: format!(
                                    "Unknown theme '{}'. Valid: auto, dark, light, dark-daltonized, light-daltonized, dark-ansi, light-ansi",
                                    args
                                ),
                            });
                        }
                        continue;
                    }

                    // Try to handle as a slash command first
                    if text.starts_with('/') {
                        match self.try_execute_command(&text) {
                            Some(CommandAction::Display(output)) => {
                                // Check if it's the special theme-picker signal
                                if output == "__open_theme_picker__" {
                                    self.prompt.clear();
                                    self.theme_picker.open(self.theme_setting);
                                    continue;
                                }
                                // Action-type command: show output as system message
                                self.message_list
                                    .push(MessageEntry::System { text: output });

                                // Extract side-effect flags and drop the guard
                                // before awaiting engine_tx.send — holding a
                                // std::sync::Mutex across .await can deadlock
                                // the runtime (clippy::await_holding_lock).
                                let side_effects =
                                    self.shared_state.lock().ok().map(|mut state| {
                                        let clear = state.clear_requested;
                                        let fork = state.fork_requested;
                                        if clear {
                                            state.clear_requested = false;
                                        }
                                        if fork {
                                            state.fork_requested = false;
                                        }
                                        (clear, fork, state.brief_mode)
                                    });
                                if let Some((clear_requested, _fork_requested, brief_mode)) =
                                    side_effects
                                {
                                    if clear_requested {
                                        let _ = engine_tx
                                            .send(EngineCommand::LoadMessages(Vec::new()))
                                            .await;
                                        self.message_list = MessageList::new();
                                        self.total_tokens = 0;
                                        self.cost_tracker = CostTracker::new(&self.model_name);
                                        self.message_list.push(MessageEntry::System {
                                            text: "Conversation history cleared.".to_string(),
                                        });
                                    }
                                    // fork_requested: side-effect flag consumed; the
                                    // engine continues with its existing history.
                                    claude_tools::brief_tool::set_brief_mode(brief_mode);
                                }

                                continue;
                            }
                            Some(CommandAction::Prompt(prompt_text)) => {
                                // Prompt-type command: inject as user message
                                self.message_list
                                    .push(MessageEntry::User { text: text.clone() });
                                let _ = engine_tx
                                    .send(EngineCommand::AddUserMessage(prompt_text))
                                    .await;
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
                        // Safety: if we get a stale/orphan permission response,
                        // recover by clearing engine_busy so the user isn't stuck.
                        if self.engine_busy && pending_tools.is_empty() {
                            tracing::warn!(
                                "Orphan PermissionResponse with no pending tools — recovering"
                            );
                            self.spinner.stop();
                            self.spinner.queued_count = 0;
                            self.engine_busy = false;
                            if let Some(queued) = self.pop_queued_message() {
                                let tx2 = tx.clone();
                                tokio::spawn(async move {
                                    let _ = tx2.send(AppEvent::SubmitPrompt(queued)).await;
                                });
                            }
                        }
                        continue;
                    }
                    let tool_idx = pending_tool_index - 1; // We already advanced past it

                    match response.as_str() {
                        "allow" | "always" => {
                            if response == "always" {
                                let mut updates =
                                    pending_tools[tool_idx].permission_updates_on_allow.clone();
                                if updates.is_empty() {
                                    updates.push(fallback_allow_update_for_tool(
                                        &pending_tools[tool_idx].info.name,
                                    ));
                                }
                                perm_ctx = apply_permission_updates(perm_ctx, &updates);
                                if let Err(err) = persist_permission_updates(&updates, &cwd) {
                                    tracing::warn!(
                                        error = %err,
                                        "failed to persist permission update"
                                    );
                                }
                            }
                            // Execute this tool in background to keep UI responsive
                            let info = &pending_tools[tool_idx].info;
                            self.message_list.set_tool_running(&info.id, true);
                            self.spinner.start(SpinnerMode::Thinking);
                            let tool_name = info.name.clone();
                            let tool_input = info.input.clone();
                            let tools_clone = tools.clone();
                            let cwd_clone = cwd.clone();
                            let cancel_clone = cancel.clone();
                            let rfs_clone = read_file_state.clone();
                            let perm_mode_clone = perm_ctx.mode.clone();
                            let model_clone = self.model_name.clone();
                            let tx_tool = tx.clone();
                            let tidx = tool_idx;
                            tokio::spawn(async move {
                                let result = execute_tool(
                                    &tools_clone,
                                    &tool_name,
                                    &tool_input,
                                    &cwd_clone,
                                    cancel_clone,
                                    rfs_clone,
                                    perm_mode_clone,
                                    &model_clone,
                                )
                                .await;
                                let mapped = result.map_err(|e| e.to_string());
                                let _ = tx_tool
                                    .send(AppEvent::ToolExecutionComplete {
                                        tool_idx: tidx,
                                        result: mapped,
                                    })
                                    .await;
                            });
                            // Don't block — continue event loop. Result comes via ToolExecutionComplete.

                            // Tool executes in background — result arrives via ToolExecutionComplete
                        }
                        "deny" => {
                            let info = &pending_tools[tool_idx].info;
                            let _ = engine_tx
                                .send(EngineCommand::AddToolResult {
                                    id: info.id.clone(),
                                    content: "Permission denied by user".to_string(),
                                    is_error: true,
                                })
                                .await;
                            self.message_list.push(MessageEntry::ToolResult {
                                name: info.name.clone(),
                                output: "Permission denied".to_string(),
                                is_error: true,
                                tool_use_id: info.id.clone(),
                            });

                            // Check next tool or continue turn
                            if pending_tool_index < pending_tools.len() {
                                self.check_next_tool_permission(
                                    &mut pending_tools,
                                    &mut pending_tool_index,
                                    &perm_ctx,
                                    &tools,
                                    &engine_tx,
                                    &tx,
                                )
                                .await;
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

                AppEvent::PermissionExplanation(text) => {
                    if let Some(ref mut dialog) = self.permission_dialog {
                        dialog.set_explanation(text);
                    }
                }

                AppEvent::ToolExecutionComplete { tool_idx, result } => {
                    self.spinner.stop();

                    if tool_idx >= pending_tools.len() {
                        // Safety: stale tool completion — recover
                        tracing::warn!(
                            tool_idx,
                            "ToolExecutionComplete for unknown tool_idx — recovering"
                        );
                        if self.engine_busy {
                            self.engine_busy = false;
                            self.spinner.queued_count = 0;
                            if let Some(queued) = self.pop_queued_message() {
                                let tx2 = tx.clone();
                                tokio::spawn(async move {
                                    let _ = tx2.send(AppEvent::SubmitPrompt(queued)).await;
                                });
                            }
                        }
                        continue;
                    }
                    let info = &pending_tools[tool_idx].info;
                    self.message_list.set_tool_running(&info.id, false);

                    // Update working directory for worktree tools
                    if let Ok(ref data) = result {
                        if !data.is_error {
                            match info.name.as_str() {
                                "EnterWorktree" => {
                                    if let Some(path) = data.data["worktreePath"].as_str() {
                                        cwd = PathBuf::from(path);
                                    }
                                }
                                "ExitWorktree" => {
                                    cwd = original_cwd.clone();
                                }
                                _ => {}
                            }
                        }
                    }

                    // Check for AskUserQuestion awaiting_input
                    let awaiting = result
                        .as_ref()
                        .ok()
                        .and_then(|d| d.data["awaiting_input"].as_bool())
                        .unwrap_or(false);

                    if awaiting {
                        let question = result
                            .as_ref()
                            .ok()
                            .and_then(|d| d.data["question"].as_str())
                            .unwrap_or("")
                            .to_string();
                        let opts: Vec<String> = result
                            .as_ref()
                            .ok()
                            .and_then(|d| d.data["options"].as_array())
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|v| v.as_str().map(String::from))
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
                        let (mut result_text, mut display_text, mut is_error, mut result_json) =
                            match &result {
                                Ok(data) => {
                                    let raw = data
                                        .data
                                        .as_str()
                                        .unwrap_or(&data.data.to_string())
                                        .to_string();
                                    let display =
                                        format_tool_result_display(&info.name, &data.data, &raw);
                                    (raw, display, data.is_error, data.data.clone())
                                }
                                Err(e) => {
                                    let msg = format!("Error: {}", e);
                                    (
                                        msg.clone(),
                                        msg.clone(),
                                        true,
                                        serde_json::json!({"error": msg}),
                                    )
                                }
                            };
                        if let Some(runner) = get_global_runner() {
                            if is_error {
                                let failure = run_post_tool_use_failure_hooks(
                                    &runner,
                                    &info.name,
                                    &info.id,
                                    &info.input,
                                    &result_text,
                                    None,
                                    Some(permission_mode_hook_name(&perm_ctx.mode)),
                                    None,
                                    None,
                                )
                                .await;
                                for context in &failure.additional_contexts {
                                    let _ = engine_tx
                                        .send(EngineCommand::AddUserContext(context.clone()))
                                        .await;
                                }
                                if let Some(message) =
                                    hook_blocking_errors_text(&failure.blocking_errors)
                                {
                                    result_text = message.clone();
                                    display_text = message;
                                    is_error = true;
                                }
                            } else {
                                let post = run_post_tool_use_hooks(
                                    &runner,
                                    &info.name,
                                    &info.id,
                                    &info.input,
                                    &result_json,
                                    Some(permission_mode_hook_name(&perm_ctx.mode)),
                                    None,
                                    None,
                                )
                                .await;
                                for context in &post.additional_contexts {
                                    let _ = engine_tx
                                        .send(EngineCommand::AddUserContext(context.clone()))
                                        .await;
                                }
                                if let Some(updated) = post.updated_mcp_tool_output {
                                    result_json = updated;
                                    result_text = result_json
                                        .as_str()
                                        .unwrap_or(&result_json.to_string())
                                        .to_string();
                                    display_text = format_tool_result_display(
                                        &info.name,
                                        &result_json,
                                        &result_text,
                                    );
                                }
                                if let Some(message) =
                                    hook_blocking_errors_text(&post.blocking_errors)
                                {
                                    result_text = message.clone();
                                    display_text = message;
                                    is_error = true;
                                } else if post.prevent_continuation {
                                    let message = post.stop_reason.unwrap_or_else(|| {
                                        "PostToolUse hook stopped continuation".to_string()
                                    });
                                    result_text = message.clone();
                                    display_text = message;
                                    is_error = true;
                                }
                            }
                        }
                        let _ = engine_tx
                            .send(EngineCommand::AddToolResult {
                                id: info.id.clone(),
                                content: result_text,
                                is_error,
                            })
                            .await;
                        if !is_error {
                            let touched_paths = dynamic_skill_file_paths(&info.name, &info.input);
                            if !touched_paths.is_empty() {
                                let skill_dirs =
                                    skill::discover_skill_dirs_for_paths(&touched_paths, &cwd);
                                let mut newly_available = skill::add_skill_directories(&skill_dirs);
                                newly_available.extend(
                                    skill::activate_conditional_skills_for_paths(
                                        &touched_paths,
                                        &cwd,
                                    ),
                                );
                                if !newly_available.is_empty() {
                                    let mut seen = self
                                        .skills
                                        .iter()
                                        .map(|skill| skill.name.clone())
                                        .collect::<std::collections::HashSet<_>>();
                                    let mut unique_new = Vec::new();
                                    for skill in newly_available {
                                        if !skill.disable_model_invocation
                                            && seen.insert(skill.name.clone())
                                        {
                                            claude_tools::skill_tool::register_discovered_skill(
                                                &skill,
                                            );
                                            unique_new.push(skill.clone());
                                            self.skills.push(skill);
                                        }
                                    }
                                    if !unique_new.is_empty() {
                                        let _ = engine_tx
                                            .send(EngineCommand::AddUserContextMessage(
                                                skills_reminder_block(&unique_new),
                                            ))
                                            .await;
                                    }
                                }
                            }
                        }
                        self.message_list.push(MessageEntry::ToolResult {
                            name: info.name.clone(),
                            output: truncate_result(&display_text),
                            is_error,
                            tool_use_id: info.id.clone(),
                        });

                        // Check next tool or continue turn
                        pending_tool_index = tool_idx + 1;
                        if pending_tool_index < pending_tools.len() {
                            self.check_next_tool_permission(
                                &mut pending_tools,
                                &mut pending_tool_index,
                                &perm_ctx,
                                &tools,
                                &engine_tx,
                                &tx,
                            )
                            .await;
                        } else {
                            let tx2 = tx.clone();
                            tokio::spawn(async move {
                                let _ = tx2.send(AppEvent::ContinueTurn).await;
                            });
                        }
                    }
                }

                AppEvent::TurnComplete(result) => {
                    match result {
                        Ok(TurnResult::Done(_stop_reason)) => {
                            self.spinner.stop();
                            self.spinner.queued_count = 0;
                            self.engine_busy = false;

                            // Dispatch any message that was queued while busy
                            if let Some(queued) = self.pop_queued_message() {
                                let tx2 = tx.clone();
                                tokio::spawn(async move {
                                    let _ = tx2.send(AppEvent::SubmitPrompt(queued)).await;
                                });
                            }
                        }
                        Ok(TurnResult::MaxTurns { max_turns, .. }) => {
                            self.spinner.stop();
                            self.spinner.queued_count = 0;
                            self.engine_busy = false;
                            self.message_list.push(MessageEntry::System {
                                text: format!("Reached maximum number of turns ({max_turns})"),
                            });

                            if let Some(queued) = self.pop_queued_message() {
                                let tx2 = tx.clone();
                                tokio::spawn(async move {
                                    let _ = tx2.send(AppEvent::SubmitPrompt(queued)).await;
                                });
                            }
                        }
                        Ok(TurnResult::ToolUse(tool_uses)) => {
                            pending_tools = tool_uses
                                .into_iter()
                                .map(|info| PendingTool {
                                    info,
                                    permission_updates_on_allow: Vec::new(),
                                })
                                .collect();
                            pending_tool_index = 0;
                            self.spinner.stop();
                            self.check_next_tool_permission(
                                &mut pending_tools,
                                &mut pending_tool_index,
                                &perm_ctx,
                                &tools,
                                &engine_tx,
                                &tx,
                            )
                            .await;
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
                            self.spinner.queued_count = 0;
                            self.engine_busy = false;
                            self.message_list.push(MessageEntry::System {
                                text: format!("Error: {}", e),
                            });

                            // Dispatch any message that was queued while busy
                            if let Some(queued) = self.pop_queued_message() {
                                let tx2 = tx.clone();
                                tokio::spawn(async move {
                                    let _ = tx2.send(AppEvent::SubmitPrompt(queued)).await;
                                });
                            }
                        }
                    }
                }
            }
        }

        // Cleanup
        terminal::disable_raw_mode()?;
        execute!(io::stdout(), LeaveAlternateScreen, cursor::Show)?;
        Ok(())
    }

    /// Check permissions for the next tool in the pending list.
    /// If the tool is auto-allowed, execute it immediately and advance.
    /// If it needs user permission, show the dialog.
    async fn check_next_tool_permission(
        &mut self,
        pending_tools: &mut [PendingTool],
        pending_tool_index: &mut usize,
        perm_ctx: &ToolPermissionContext,
        tools: &ToolRegistry,
        engine_tx: &mpsc::Sender<EngineCommand>,
        tx: &mpsc::Sender<AppEvent>,
    ) {
        if *pending_tool_index < pending_tools.len() {
            let tool_idx = *pending_tool_index;
            *pending_tool_index += 1;
            let tool_name = pending_tools[tool_idx].info.name.clone();
            let tool_id = pending_tools[tool_idx].info.id.clone();
            let mut tool_input = pending_tools[tool_idx].info.input.clone();
            let mut forced_permission: Option<Result<(), String>> = None;

            if let Some(runner) = get_global_runner() {
                let pre = run_pre_tool_use_hooks(
                    &runner,
                    &tool_name,
                    &tool_id,
                    &tool_input,
                    Some(permission_mode_hook_name(&perm_ctx.mode)),
                    None,
                    None,
                )
                .await;
                for context in &pre.additional_contexts {
                    let _ = engine_tx
                        .send(EngineCommand::AddUserContext(context.clone()))
                        .await;
                }
                if let Some(message) =
                    hook_blocking_errors_text(&pre.blocking_errors).or(pre.denial_message.clone())
                {
                    forced_permission = Some(Err(message));
                } else if pre.prevent_continuation {
                    forced_permission = Some(Err(pre
                        .stop_reason
                        .unwrap_or_else(|| "PreToolUse hook stopped tool execution".to_string())));
                } else {
                    let resolved =
                        resolve_hook_permission_decision(&pre, &tool_input, |candidate_input| {
                            let tool_name = tool_name.clone();
                            let perm_ctx = perm_ctx.clone();
                            let candidate_input = candidate_input.clone();
                            async move {
                                let decision = if let Some(tool) = tools.get(&tool_name) {
                                    let tool_perms =
                                        claude_tools::registry::ExecutorToolPermissions::new(
                                            tool,
                                            candidate_input.clone(),
                                        );
                                    evaluate_permission(&tool_perms, &candidate_input, &perm_ctx)
                                } else {
                                    let tool_perms =
                                        claude_core::permissions::evaluator::SimpleToolPermissions::new(
                                            &tool_name,
                                            false,
                                        );
                                    evaluate_permission(&tool_perms, &candidate_input, &perm_ctx)
                                };
                                permission_decision_to_rule_check(&decision)
                            }
                        })
                        .await;
                    match resolved {
                        ResolvedPermission::Allow { updated_input }
                        | ResolvedPermission::NormalFlow { updated_input } => {
                            tool_input = merge_hook_updated_input(&tool_input, &updated_input);
                        }
                        ResolvedPermission::Deny { message } => {
                            forced_permission = Some(Err(message.unwrap_or_else(|| {
                                "PreToolUse hook denied this tool".to_string()
                            })));
                        }
                        ResolvedPermission::RequiresUserConfirmation {
                            updated_input,
                            force_decision,
                        } => {
                            tool_input = merge_hook_updated_input(&tool_input, &updated_input);
                            forced_permission = Some(Err(force_decision
                                .unwrap_or_else(|| "Tool requires user confirmation".to_string())));
                        }
                    }
                }
            }
            pending_tools[tool_idx].info.input = tool_input.clone();

            // Determine if tool is read-only
            let is_read_only = tools
                .get(&tool_name)
                .map(|t| t.is_read_only(&tool_input))
                .unwrap_or(false);

            let decision = if let Some(forced) = forced_permission {
                match forced {
                    Ok(()) => PermissionDecision::allow(),
                    Err(message) => PermissionDecision::deny(
                        message,
                        PermissionDecisionReason::Hook {
                            hook_name: format!("PreToolUse:{tool_name}"),
                            hook_source: None,
                            reason: None,
                        },
                    ),
                }
            } else {
                if let Some(tool) = tools.get(&tool_name) {
                    let tool_perms = claude_tools::registry::ExecutorToolPermissions::new(
                        tool,
                        tool_input.clone(),
                    );
                    evaluate_permission(&tool_perms, &tool_input, perm_ctx)
                } else {
                    use claude_core::permissions::evaluator::SimpleToolPermissions;
                    let tool_perms = SimpleToolPermissions::new(&tool_name, is_read_only);
                    evaluate_permission(&tool_perms, &tool_input, perm_ctx)
                }
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
                }
                PermissionDecision::Ask(ask) => {
                    let input_preview = serde_json::to_string_pretty(&tool_input)
                        .unwrap_or_else(|_| tool_input.to_string());
                    pending_tools[tool_idx].permission_updates_on_allow = ask
                        .suggestions
                        .clone()
                        .unwrap_or_else(|| vec![fallback_allow_update_for_tool(&tool_name)]);
                    let tool_name_for_explainer = tool_name.clone();
                    let description_for_explainer = ask.message.clone();
                    let preview_for_explainer = input_preview.clone();
                    self.permission_dialog = Some(PermissionDialog::new(
                        tool_name.clone(),
                        ask.message,
                        input_preview,
                    ));
                    // Fire the Haiku explainer in the background; result
                    // arrives via AppEvent::PermissionExplanation. No-op
                    // when no secondary model is registered.
                    let tx_explain = tx.clone();
                    tokio::spawn(async move {
                        let text = PermissionDialog::fetch_explanation(
                            &tool_name_for_explainer,
                            &description_for_explainer,
                            &preview_for_explainer,
                            "",
                        )
                        .await;
                        let _ = tx_explain.send(AppEvent::PermissionExplanation(text)).await;
                    });
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
                }
            }
        }
    }

    fn handle_stream_event(&mut self, event: StreamEvent) {
        match event {
            StreamEvent::TextDelta { text } => {
                self.spinner.set_mode(SpinnerMode::Responding);
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
                self.spinner.set_mode(SpinnerMode::Thinking);
                if let Some(MessageEntry::Thinking { text: ref mut t }) =
                    self.message_list.messages_mut().last_mut()
                {
                    t.push_str(&text);
                } else {
                    self.message_list.push(MessageEntry::Thinking { text });
                }
            }
            StreamEvent::ToolStart {
                tool_use_id,
                name,
                input,
            } => {
                let summary = format_tool_use_summary(&name, &input);
                self.message_list.push(MessageEntry::ToolUse {
                    name: name.clone(),
                    input_summary: summary,
                    tool_use_id,
                });
                self.spinner.set_mode(SpinnerMode::ToolUse);
            }
            StreamEvent::ToolResult {
                tool_use_id,
                result,
            } => {
                self.message_list.push(MessageEntry::ToolResult {
                    name: "tool".to_string(),
                    output: truncate_result(
                        result.data.as_str().unwrap_or(&result.data.to_string()),
                    ),
                    is_error: result.is_error,
                    tool_use_id,
                });
            }
            StreamEvent::Done { stop_reason: _ } => {
                self.spinner.stop();
            }
            StreamEvent::UsageUpdate(ref usage) => {
                // Track the latest turn's input_tokens for context window
                // display. MessageStart carries input_tokens (representing
                // how full the context is); MessageDelta carries 0 input.
                if usage.input_tokens > 0 {
                    self.total_tokens = usage.input_tokens;
                    self.spinner.input_tokens = usage.input_tokens;
                }
                self.spinner.output_tokens = usage.output_tokens;
                self.cost_tracker.add_usage(usage);
                // Sync shared state for slash commands
                if let Ok(mut state) = self.shared_state.lock() {
                    state.total_tokens = self.total_tokens;
                    state.request_count = self.cost_tracker.request_count();
                    state.total_cost_usd = self.cost_tracker.total_cost_usd();
                }
            }
            StreamEvent::RequestStart { request_id: _ } => {
                self.spinner.start(SpinnerMode::Requesting);
                self.cost_tracker.increment_request_count();
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
            MouseEventKind::Down(MouseButton::Left) => {
                if self.permission_dialog.is_some() || self.ask_user_dialog.is_some() {
                    return;
                }
                let Ok(area) = self.terminal.size() else {
                    return;
                };
                let spinner_height = if self.spinner.active { 1 } else { 0 };
                let messages_height = area.height.saturating_sub(spinner_height + 3);
                if mouse.row >= messages_height {
                    return;
                }
                if let Some(tool_use_id) = self.message_list.tool_result_at_row(
                    mouse.row as usize,
                    area.width,
                    messages_height as usize,
                    &self.theme,
                ) {
                    self.message_list.toggle_tool_expand(&tool_use_id);
                    self.message_list.scroll_to_bottom();
                }
            }
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
        let rewind_picker = &self.rewind_picker;
        let rewind_confirmation = &self.rewind_confirmation;
        let message_queue = &self.message_queue;
        let model_picker = &self.model_picker;
        let theme_picker = &self.theme_picker;
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
            let queued_message_visible = engine_busy && !message_queue.is_empty();
            let spinner_height = if spinner.active { 1 } else { 0 };
            let activity_height = spinner_height + if queued_message_visible { 2 } else { 0 };
            let chunks = Layout::default()
                .constraints([
                    Constraint::Min(1),                  // Messages
                    Constraint::Length(activity_height), // Spinner + queued message preview
                    Constraint::Length(3),               // Input (border + input + border)
                ])
                .split(area);

            // Messages area — pass theme for correct colors
            let msg_widget = MessageListWidget::new(message_list).theme(theme);
            frame.render_widget(msg_widget, chunks[0]);

            // Spinner (inline with messages, like the original)
            if spinner.active {
                let spinner_area = Rect::new(chunks[1].x, chunks[1].y, chunks[1].width, 1);
                frame.render_widget(spinner, spinner_area);
            }

            if queued_message_visible {
                let queue_y = chunks[1].y + spinner_height + 1;
                if queue_y < chunks[1].y + chunks[1].height {
                    let preview = truncate_chars(message_queue[0].replace('\n', " ").trim(), 160);
                    let queued_line = Line::from(vec![
                        Span::raw("  "),
                        Span::styled("\u{276F} ", Style::default().fg(theme.permission)),
                        Span::styled(preview, Style::default().fg(theme.text)),
                    ]);
                    frame.buffer_mut().set_line(
                        chunks[1].x,
                        queue_y,
                        &queued_line,
                        chunks[1].width,
                    );
                }
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
                let status = fit_status_for_width(&status, input_area.width as usize);

                // Build the top border with status text embedded:
                // ── status text ──────────
                let status_width = status.chars().count();
                let left_dashes = 2usize.min(input_area.width as usize);
                let remaining =
                    (input_area.width as usize).saturating_sub(left_dashes + status_width + 2);
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

                    // Multi-line aware rendering: split on '\n' and render each line.
                    let text_str = if queued_message_visible {
                        "Press up to edit queued messages".to_string()
                    } else {
                        prompt.text().to_string()
                    };
                    let lines: Vec<&str> = text_str.split('\n').collect();
                    let max_visible = (input_area.height.saturating_sub(2)) as usize; // leave room for borders
                    let visible_lines = if lines.len() > max_visible && max_visible > 0 {
                        &lines[lines.len() - max_visible..]
                    } else {
                        &lines[..]
                    };
                    for (i, line_text) in visible_lines.iter().enumerate() {
                        let prefix = if i == 0 { "\u{276F} " } else { "  " };
                        let line = ratatui::text::Line::from(vec![
                            ratatui::text::Span::styled(prefix, prompt_style),
                            ratatui::text::Span::raw(line_text.to_string()),
                        ]);
                        let y = input_area.y + 1 + i as u16;
                        if y < input_area.y + input_area.height.saturating_sub(1) {
                            buf.set_line(input_area.x, y, &line, input_area.width);
                        }
                    }

                    // Bottom border
                    let bottom_y = input_area.y + input_area.height.saturating_sub(1);
                    let bottom_line = ratatui::text::Line::from(ratatui::text::Span::styled(
                        "\u{2500}".repeat(input_area.width as usize),
                        ratatui::style::Style::default().fg(border_color),
                    ));
                    buf.set_line(input_area.x, bottom_y, &bottom_line, input_area.width);
                } else {
                    // Minimal: show only the last line of multi-line input
                    let prompt_style = if engine_busy {
                        ratatui::style::Style::default()
                            .fg(border_color)
                            .add_modifier(ratatui::style::Modifier::DIM)
                    } else {
                        ratatui::style::Style::default()
                    };
                    let text_str = if queued_message_visible {
                        "Press up to edit queued messages".to_string()
                    } else {
                        prompt.text().to_string()
                    };
                    let last_line = text_str.split('\n').next_back().unwrap_or("");
                    let prompt_line = ratatui::text::Line::from(vec![
                        ratatui::text::Span::styled("\u{276F} ", prompt_style),
                        ratatui::text::Span::raw(last_line.to_string()),
                    ]);
                    buf.set_line(input_area.x, input_area.y, &prompt_line, input_area.width);
                }
            }

            // Command picker overlay (positioned above the input area)
            if command_picker.visible {
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

            if rewind_picker.visible {
                let picker_height = ((rewind_picker.filtered_count() as u16) + 2)
                    .max(3)
                    .min(area.height * 2 / 3);
                let picker_area = Rect::new(
                    area.x + 1,
                    chunks[2].y.saturating_sub(picker_height),
                    area.width.saturating_sub(2),
                    picker_height,
                );
                let picker_widget = CommandPickerWidget::new(rewind_picker).titled("Rewind", "");
                frame.render_widget(picker_widget, picker_area);
            }

            if let Some(confirm) = rewind_confirmation {
                let confirm_height = 14u16.min(area.height.saturating_sub(1)).max(3);
                let confirm_area = Rect::new(
                    area.x + 1,
                    chunks[2].y.saturating_sub(confirm_height),
                    area.width.saturating_sub(2),
                    confirm_height,
                );
                render_rewind_confirmation(frame, confirm_area, confirm, theme);
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

            // Theme picker — rendered inline above the prompt (same pattern as model picker)
            if theme_picker.visible {
                let picker_height = theme_picker.height().min(area.height.saturating_sub(4));
                let picker_area = Rect::new(
                    area.x,
                    chunks[2].y.saturating_sub(picker_height),
                    area.width,
                    picker_height,
                );
                let picker_widget = ThemePickerWidget::new(theme_picker);
                frame.render_widget(picker_widget, picker_area);
            }

            // Permission dialog overlay
            if let Some(dialog) = permission_dialog {
                let dialog_height = dialog.height().min(area.height.saturating_sub(3)).max(4);
                let dialog_area = Rect::new(
                    area.x,
                    chunks[2].y.saturating_sub(dialog_height),
                    area.width,
                    dialog_height,
                );
                frame.render_widget(dialog, dialog_area);
            }

            // AskUser input dialog overlay
            if let Some(ref ask_dialog) = ask_user_dialog {
                let dialog_area = centered_rect(70, 10, area);
                frame.render_widget(ask_dialog, dialog_area);
            }

            // Show cursor at input position when no dialog is active.
            // Multi-line aware: compute row/col from cursor byte offset.
            if permission_dialog.is_none() && ask_user_dialog.is_none() && !engine_busy {
                let text_before_cursor = &prompt.text()[..prompt.cursor()];
                let lines_before: Vec<&str> = text_before_cursor.split('\n').collect();
                let cursor_row = lines_before.len().saturating_sub(1);
                let cursor_col_chars = lines_before.last().map_or(0, |l| l.chars().count());

                // Account for visible line scrolling (same logic as render)
                let text_str = prompt.text().to_string();
                let total_lines = text_str.split('\n').count();
                let max_visible = (input_area.height.saturating_sub(2)) as usize;
                let scroll_offset = if total_lines > max_visible && max_visible > 0 {
                    total_lines - max_visible
                } else {
                    0
                };
                let visible_row = cursor_row.saturating_sub(scroll_offset);

                let cursor_x = input_area.x + 2 + cursor_col_chars as u16; // 2 = "❯ " or "  "
                let cursor_y = input_area.y + 1 + visible_row as u16;
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
        let _ = execute!(io::stdout(), LeaveAlternateScreen, cursor::Show);
    }
}

/// Execute a tool call.
///
/// Constructs the `ToolUseContext` explicitly with the session's
/// live model and a `NullToolHost`. Not `for_test` — the CLI/TUI
/// production path isn't a test, and burying the model hard-code
/// in a factory would quietly regress any tool/command that reads
/// `ctx.options.main_loop_model` (e.g. the command adapter at
/// `claude-core/src/command_adapter.rs:114`).
async fn execute_tool(
    tools: &ToolRegistry,
    name: &str,
    input: &serde_json::Value,
    cwd: &std::path::Path,
    cancel: CancellationToken,
    read_file_state: std::sync::Arc<std::sync::Mutex<claude_tools::registry::ReadFileState>>,
    permission_mode: PermissionMode,
    model: &str,
) -> Result<ToolResultData> {
    use claude_core::tool_host::NullToolHost;
    use claude_core::tool_use_context_options::ToolUseContextOptions;

    let executor = tools
        .get(name)
        .ok_or_else(|| anyhow::anyhow!("Unknown tool: {}", name))?;
    let options = Arc::new(ToolUseContextOptions::minimal(model));
    let host = Arc::new(NullToolHost);
    let ctx = ToolUseContext::new(
        cwd.to_path_buf(),
        read_file_state,
        permission_mode,
        options,
        host,
    );
    executor.call(input, &ctx, cancel, None).await
}

/// Format a tool result for display, extracting relevant content per tool.
/// Mirrors TS per-tool renderToolResultMessage() functions.
fn format_tool_result_display(tool_name: &str, data: &serde_json::Value, raw: &str) -> String {
    match tool_name {
        "Bash" | "PowerShell" => {
            // TS BashToolResultMessage: shows stdout, stderr separately
            let stdout = data["stdout"].as_str().unwrap_or("");
            let stderr = data["stderr"].as_str().unwrap_or("");
            let code = data["code"].as_i64().unwrap_or(0);
            let mut parts = Vec::new();
            if !stdout.is_empty() {
                parts.push(stdout.to_string());
            }
            if !stderr.is_empty() {
                parts.push(format!("stderr: {}", stderr));
            }
            if parts.is_empty() {
                if code != 0 {
                    format!("(exit code {})", code)
                } else {
                    "(no output)".to_string()
                }
            } else {
                parts.join("\n")
            }
        }
        "Read" => {
            // TS: "Read N lines"
            if let Some(content) = data["file"].as_object() {
                if let Some(n) = content.get("numLines").and_then(|v| v.as_u64()) {
                    return format!("Read {} {}", n, if n == 1 { "line" } else { "lines" });
                }
                if let Some(content_str) = content.get("content").and_then(|v| v.as_str()) {
                    let n = content_str.lines().count();
                    return format!("Read {} {}", n, if n == 1 { "line" } else { "lines" });
                }
            }
            // Fallback: count lines in raw content
            if let Some(content) = data.as_str() {
                let n = content.lines().count();
                format!("Read {} {}", n, if n == 1 { "line" } else { "lines" })
            } else {
                raw.to_string()
            }
        }
        "Edit" | "Write" => format_edit_result(data).unwrap_or_else(|| raw.to_string()),
        "Glob" => {
            // TS: show file count and list
            if let Some(files) = data["filenames"].as_array() {
                let truncated = data["truncated"].as_bool().unwrap_or(false);
                let suffix = if truncated { "+" } else { "" };
                format!("{}{} files", files.len(), suffix)
            } else {
                raw.to_string()
            }
        }
        "Grep" => {
            // TS: show file count or match count
            if let Some(files) = data["filenames"].as_array() {
                let mode = data["mode"].as_str().unwrap_or("files_with_matches");
                match mode {
                    "count" => format!("{} files with matches", files.len()),
                    "content" => {
                        if let Some(content) = data["content"].as_str() {
                            content.to_string()
                        } else {
                            format!("{} files", files.len())
                        }
                    }
                    _ => format!("{} files", files.len()),
                }
            } else {
                raw.to_string()
            }
        }
        "Agent" => {
            // TS: shows agent result summary
            if let Some(result) = data["result"].as_str() {
                result.to_string()
            } else {
                raw.to_string()
            }
        }
        "TodoWrite" => {
            // Show a clean confirmation
            if let Some(msg) = data["message"].as_str() {
                msg.to_string()
            } else {
                "Todos updated".to_string()
            }
        }
        "REPL" => {
            // Show stdout from REPL execution
            let stdout = data["stdout"].as_str().unwrap_or("");
            let stderr = data["stderr"].as_str().unwrap_or("");
            if !stdout.is_empty() {
                stdout.to_string()
            } else if !stderr.is_empty() {
                stderr.to_string()
            } else {
                "(no output)".to_string()
            }
        }
        "AskUserQuestion" | "AskUser" => {
            // Don't show raw JSON for AskUser
            if let Some(q) = data["question"].as_str() {
                q.to_string()
            } else {
                raw.to_string()
            }
        }
        _ => {
            // Default: if it's a JSON object, try to extract a "result" or "message" field
            if let Some(msg) = data["message"].as_str() {
                msg.to_string()
            } else if let Some(result) = data["result"].as_str() {
                result.to_string()
            } else if data.is_string() {
                data.as_str().unwrap_or(raw).to_string()
            } else {
                raw.to_string()
            }
        }
    }
}

/// Format a tool use input into a clean summary like the TS UI.
/// e.g., Read(file_path · lines 10-20) instead of raw JSON.
fn format_tool_use_summary(name: &str, input: &serde_json::Value) -> String {
    match name {
        "Read" => {
            let path = input["file_path"].as_str().unwrap_or("?");
            let display = shorten_path(path);
            let mut parts = vec![display.to_string()];
            if let Some(offset) = input["offset"].as_u64() {
                if let Some(limit) = input["limit"].as_u64() {
                    parts.push(format!("lines {}-{}", offset, offset + limit - 1));
                } else {
                    parts.push(format!("from line {}", offset));
                }
            }
            if let Some(pages) = input["pages"].as_str() {
                parts.push(format!("pages {}", pages));
            }
            parts.join(" · ")
        }
        "Edit" => {
            let path = input["file_path"].as_str().unwrap_or("?");
            shorten_path(path).to_string()
        }
        "Write" => {
            let path = input["file_path"].as_str().unwrap_or("?");
            shorten_path(path).to_string()
        }
        "Bash" => {
            let cmd = input["command"].as_str().unwrap_or("?");
            if cmd.chars().count() > 100 {
                format!("{}...", truncate_chars(cmd, 97))
            } else {
                cmd.to_string()
            }
        }
        "Glob" => {
            let pattern = input["pattern"].as_str().unwrap_or("?");
            pattern.to_string()
        }
        "Grep" => {
            let pattern = input["pattern"].as_str().unwrap_or("?");
            let path = input["path"].as_str().unwrap_or("");
            if path.is_empty() {
                format!("\"{}\"", pattern)
            } else {
                format!("\"{}\" in {}", pattern, shorten_path(path))
            }
        }
        "Agent" => {
            let desc = input["description"].as_str().unwrap_or("");
            let subtype = input["subagent_type"].as_str().unwrap_or("");
            if !desc.is_empty() {
                desc.to_string()
            } else if !subtype.is_empty() {
                subtype.to_string()
            } else {
                let prompt = input["prompt"].as_str().unwrap_or("?");
                if prompt.chars().count() > 80 {
                    format!("{}...", truncate_chars(prompt, 77))
                } else {
                    prompt.to_string()
                }
            }
        }
        _ => {
            // Fallback: compact JSON, truncated
            let s = serde_json::to_string(input).unwrap_or_else(|_| input.to_string());
            if s.chars().count() > 120 {
                format!("{}...", truncate_chars(&s, 117))
            } else {
                s
            }
        }
    }
}

/// Shorten a file path by keeping only the last 2 components.
fn shorten_path(path: &str) -> &str {
    let parts: Vec<&str> = path.rsplitn(3, '/').collect();
    if parts.len() >= 2 {
        // rsplitn gives [filename, parent, rest...], we want parent/filename
        let start = path.len() - parts[0].len() - parts[1].len() - 1;
        &path[start..]
    } else {
        path
    }
}

/// Convert Edit/Write tool JSON result into a readable diff string.
/// Returns None if the data isn't an edit result.
fn format_edit_result(data: &serde_json::Value) -> Option<String> {
    let file_path = data["filePath"].as_str()?;
    let old_string = data["oldString"].as_str().unwrap_or("");
    let new_string = data["newString"].as_str().unwrap_or("");

    if old_string.is_empty() && new_string.is_empty() {
        return None;
    }

    let mut output = format!("--- {}\n+++ {}\n", file_path, file_path);

    // Generate unified diff from old/new strings
    let old_lines: Vec<&str> = if old_string.is_empty() {
        Vec::new()
    } else {
        old_string.lines().collect()
    };
    let new_lines: Vec<&str> = if new_string.is_empty() {
        Vec::new()
    } else {
        new_string.lines().collect()
    };

    output.push_str(&format!(
        "@@ -{},{} +{},{} @@\n",
        1,
        old_lines.len(),
        1,
        new_lines.len()
    ));

    for line in &old_lines {
        output.push_str(&format!("-{}\n", line));
    }
    for line in &new_lines {
        output.push_str(&format!("+{}\n", line));
    }

    Some(output)
}

/// Truncate long tool results for display.
fn truncate_result(s: &str) -> String {
    const MAX_DISPLAY: usize = 2000;
    if s.chars().count() <= MAX_DISPLAY {
        s.to_string()
    } else {
        let total = s.chars().count();
        format!(
            "{}... ({} chars total)",
            truncate_chars(s, MAX_DISPLAY),
            total
        )
    }
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}

fn parse_rewind_picker_name(name: &str) -> Option<usize> {
    name.strip_prefix('#')?.parse().ok()
}

fn reconstruct_engine_messages(entries: &[MessageEntry]) -> Vec<serde_json::Value> {
    entries
        .iter()
        .filter_map(|entry| match entry {
            MessageEntry::User { text } => Some(serde_json::json!({
                "role": "user",
                "content": [{"type": "text", "text": text}]
            })),
            MessageEntry::Assistant { text } => Some(serde_json::json!({
                "role": "assistant",
                "content": [{"type": "text", "text": text}]
            })),
            MessageEntry::CompactionSummary { text } => Some(serde_json::json!({
                "role": "user",
                "content": [{"type": "text", "text": text}]
            })),
            _ => None,
        })
        .collect()
}

fn message_text(message: &serde_json::Value) -> Option<String> {
    let content = message.get("content")?;
    match content {
        serde_json::Value::String(text) => Some(text.clone()),
        serde_json::Value::Array(blocks) => Some(
            blocks
                .iter()
                .filter_map(|block| block.get("text").and_then(|text| text.as_str()))
                .collect::<Vec<_>>()
                .join(""),
        ),
        _ => None,
    }
}

fn fit_status_for_width(status: &str, width: usize) -> String {
    let max_status = width.saturating_sub(4);
    let len = status.chars().count();
    if len <= max_status {
        return status.to_string();
    }
    if max_status == 0 {
        return String::new();
    }
    if max_status <= 3 {
        return truncate_chars(status, max_status);
    }
    format!("{}...", truncate_chars(status, max_status - 3))
}

/// Calculate a centered rect within the given area.
fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let width = (area.width * percent_x / 100).max(1).min(area.width);
    let height = height.max(1).min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width, height)
}

fn render_rewind_confirmation(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    confirm: &RewindConfirmation,
    theme: &Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let prompt = truncate_chars(confirm.prompt_text.replace('\n', " ").trim(), 96);
    let mut lines = vec![
        Line::from(Span::styled(
            "Rewind",
            Style::default()
                .fg(theme.permission)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("Confirm you want to restore to the point before you sent this message:"),
        Line::from(""),
        Line::from(vec![
            Span::styled("│ ", Style::default().fg(theme.inactive)),
            Span::styled(prompt, Style::default().fg(theme.text)),
        ]),
        Line::from(vec![
            Span::styled("│ ", Style::default().fg(theme.inactive)),
            Span::styled("(recently)", Style::default().fg(theme.inactive)),
        ]),
        Line::from(""),
        Line::from(match confirm.selected_option() {
            RewindRestoreOption::SummarizeFromHere => {
                "Messages after this point will be summarized."
            }
            RewindRestoreOption::RestoreConversation => "The conversation will be forked.",
            RewindRestoreOption::NeverMind => "The conversation will be unchanged.",
        }),
        Line::from(match confirm.selected_option() {
            RewindRestoreOption::SummarizeFromHere => "",
            _ => "The code will be unchanged.",
        }),
        Line::from(""),
    ];

    for (idx, option) in [
        RewindRestoreOption::RestoreConversation,
        RewindRestoreOption::SummarizeFromHere,
        RewindRestoreOption::NeverMind,
    ]
    .iter()
    .enumerate()
    {
        let selected = idx == confirm.selected;
        let pointer = if selected { "\u{276F} " } else { "  " };
        let style = if selected {
            Style::default()
                .fg(theme.permission)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.text)
        };
        lines.push(Line::from(vec![
            Span::styled(pointer, style),
            Span::styled(format!("{}. {}", idx + 1, option.label()), style),
        ]));
    }

    frame.render_widget(Clear, area);
    let paragraph = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(theme.permission)),
    );
    frame.render_widget(paragraph, area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_chars_preserves_utf8_boundaries() {
        assert_eq!(truncate_chars("éééabc", 4), "éééa");
    }

    #[test]
    fn fit_status_respects_width_with_unicode_separator() {
        let fitted = fit_status_for_width("claude-sonnet · $0.00 (1%) · default", 18);
        assert!(fitted.chars().count() <= 14);
        assert!(fitted.ends_with("..."));
    }

    #[test]
    fn centered_rect_has_nonzero_size_on_tiny_terminals() {
        let rect = centered_rect(60, 10, Rect::new(0, 0, 1, 1));
        assert_eq!(rect.width, 1);
        assert_eq!(rect.height, 1);
    }

    #[test]
    fn queued_messages_stay_out_of_transcript_and_can_be_edited() {
        let mut app = App::new().unwrap();
        app.enqueue_message("follow up after this".to_string());

        assert_eq!(app.spinner.queued_count, 0);
        assert_eq!(app.message_queue, vec!["follow up after this".to_string()]);
        assert!(!matches!(
            app.message_list.messages().last(),
            Some(MessageEntry::System { text }) if text.contains("Queued message:")
        ));

        assert!(app.edit_last_queued_message());
        assert_eq!(app.prompt.text(), "follow up after this");
        assert!(app.message_queue.is_empty());
        assert_eq!(app.spinner.queued_count, 0);
    }

    #[test]
    fn summarize_rewind_from_replaces_tail_and_restores_prompt() {
        let mut app = App::new().unwrap();
        app.message_list.push(MessageEntry::User {
            text: "first".to_string(),
        });
        app.message_list.push(MessageEntry::Assistant {
            text: "first answer".to_string(),
        });
        app.message_list.push(MessageEntry::User {
            text: "second".to_string(),
        });
        app.message_list.push(MessageEntry::Assistant {
            text: "second answer".to_string(),
        });

        let restored = app.summarize_rewind_from(2);

        assert_eq!(restored.as_deref(), Some("second"));
        assert_eq!(app.message_list.messages().len(), 3);
        assert!(matches!(
            app.message_list.messages().last(),
            Some(MessageEntry::CompactionSummary { text })
                if text.contains("Messages after this point were summarized")
                    && text.contains("User: second")
                    && text.contains("Assistant: second answer")
        ));

        let engine_messages = reconstruct_engine_messages(app.message_list.messages());
        assert_eq!(engine_messages.len(), 3);
        assert_eq!(engine_messages[2]["role"], "user");
    }
}
