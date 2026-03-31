use std::io::{self, Stdout};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event as CrosstermEvent, KeyCode, KeyEvent, KeyModifiers};
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

use crate::theme::{detect_theme, Theme};
use crate::widgets::message_list::{MessageEntry, MessageList, MessageListWidget};
use crate::widgets::permission_dialog::PermissionDialog;
use crate::widgets::prompt_input::{InputAction, PromptInput, PromptInputWidget};
use crate::widgets::spinner::{SpinnerMode, SpinnerState};

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
}

/// Pending tool that needs permission before execution.
struct PendingTool {
    info: ToolUseInfo,
}

pub struct App {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    theme: Theme,
    spinner: SpinnerState,
    should_quit: bool,
    message_list: MessageList,
    prompt: PromptInput,
    permission_dialog: Option<PermissionDialog>,
    /// True while the engine is processing (prevents double-submit)
    engine_busy: bool,
    /// Model name for display in the header
    model_name: String,
    /// Running total of tokens used in this session
    total_tokens: u64,
}

impl App {
    pub fn new() -> Result<Self> {
        let backend = CrosstermBackend::new(io::stdout());
        let terminal = Terminal::new(backend)?;
        Ok(Self {
            terminal,
            theme: detect_theme(),
            spinner: SpinnerState::new(),
            should_quit: false,
            message_list: MessageList::new(),
            prompt: PromptInput::new(),
            permission_dialog: None,
            engine_busy: false,
            model_name: "claude-sonnet-4-6".to_string(),
            total_tokens: 0,
        })
    }

    /// Set the model name displayed in the header.
    pub fn set_model_name(&mut self, name: &str) {
        self.model_name = name.to_string();
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

                    if self.permission_dialog.is_some() {
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
                    } else {
                        // Route keys to prompt input
                        match self.prompt.handle_key(k) {
                            InputAction::Submit(text) => {
                                let _ = tx.send(AppEvent::SubmitPrompt(text)).await;
                            }
                            InputAction::None => {}
                        }
                    }
                }
                AppEvent::SubmitPrompt(text) => {
                    if self.engine_busy || text.trim().is_empty() {
                        continue;
                    }
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
                            )
                            .await;
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
                            self.spinner.stop();

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
            StreamEvent::UsageUpdate(usage) => {
                self.spinner.tokens = usage.output_tokens;
                self.total_tokens = self.total_tokens.saturating_add(usage.output_tokens);
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
            _ => {
                self.prompt.handle_key(key);
            }
        }
    }

    fn render(&mut self) -> Result<()> {
        let theme = &self.theme;
        let spinner = &self.spinner;
        let message_list = &self.message_list;
        let prompt = &self.prompt;
        let permission_dialog = &self.permission_dialog;
        let model_name = &self.model_name;
        let total_tokens = self.total_tokens;

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

            // Header: "Claude Code (Rust) | model: ... | N tokens"
            let token_str = if total_tokens > 0 {
                format!("{} tokens", total_tokens)
            } else {
                "0 tokens".to_string()
            };
            let header = Paragraph::new(ratatui::text::Line::from(vec![
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
                    format!("model: {}", model_name),
                    ratatui::style::Style::default().fg(theme.muted),
                ),
                ratatui::text::Span::styled(
                    " | ",
                    ratatui::style::Style::default().fg(theme.border),
                ),
                ratatui::text::Span::styled(
                    token_str,
                    ratatui::style::Style::default().fg(theme.muted),
                ),
            ]));
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

            // Permission dialog overlay
            if let Some(dialog) = permission_dialog {
                let dialog_area = centered_rect(60, 10, area);
                frame.render_widget(dialog, dialog_area);
            }
        })?;
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
async fn execute_tool(
    tools: &ToolRegistry,
    name: &str,
    input: &serde_json::Value,
    cwd: &std::path::Path,
    cancel: CancellationToken,
) -> Result<ToolResultData> {
    let executor = tools
        .get(name)
        .ok_or_else(|| anyhow::anyhow!("Unknown tool: {}", name))?;
    let ctx = ToolUseContext {
        working_directory: cwd.to_path_buf(),
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
