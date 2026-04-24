//! Bridge between the legacy `commands::registry::CommandHandler`
//! and the unified `command_trait::Command` trait.
//!
//! Architecture-wiring item #2c from the post-design-review
//! punch list. The Codex CR (Q6 verdict) called for unifying
//! command execution behind one trait: "`built-in vs plugin`
//! is a loading concern, not a type-system concern."
//!
//! # Why an adapter, not a rewrite
//!
//! The existing `commands::builtin` module holds ~25 built-in
//! command implementations (`CommitPushPrHandler`,
//! `ConfigHandler`, `ModelHandler`, …), each implementing the
//! legacy [`CommandHandler`] trait. Rewriting each to the new
//! [`Command`] trait would be mechanical churn across ~25
//! files.
//!
//! An adapter is one type; it wraps any `CommandHandler` +
//! metadata and presents the new [`Command`] surface. The
//! registry can then hold a uniform `Vec<Arc<dyn Command>>`
//! where built-ins are adapter-wrapped and plugin commands
//! implement `Command` directly.
//!
//! # What the adapter handles
//!
//! 1. **Context mapping**: new [`CommandContext`] carries
//!    `args` + `&ToolUseContextOptions`; legacy expects
//!    [`LegacyCommandContext`] with `working_directory`,
//!    `model`, and optional `shared: Arc<Mutex<SharedCommandState>>`.
//!    The adapter builds a legacy context from options + the
//!    shared-state handle supplied at adapter-construction time.
//!
//! 2. **Result mapping**: legacy [`CommandResult`] has three
//!    variants (`Message` / `Action` / `Error`). New
//!    [`CommandOutcome`] has five. The adapter maps:
//!    - `Message(s)` → `CommandOutcome::Text { value: s,
//!      display: CommandResultDisplay::User }` — injected as
//!      a user message by the caller.
//!    - `Action(s)` → `CommandOutcome::Text { value: s,
//!      display: CommandResultDisplay::System }` — shown as
//!      a system message with no injection.
//!    - `Error(s)` → `Err(CommandError::Execution(s))` — the
//!      new trait signals execution failure through the
//!      `Result` arm, not a variant.
//!
//! # Scope limitations
//!
//! - The adapter does NOT plumb the new `CommandContext.options`
//!   into the legacy handler (no legacy handler reads it today).
//!   When a legacy handler starts needing options it should
//!   migrate off the adapter directly.
//! - Legacy handlers that produced JSX in the TS counterpart
//!   are represented as `CommandResult::Action(...)` text in
//!   Rust today; they map to `Text { display: System }` — the
//!   `CommandOutcome::Render` variant is reserved for future
//!   commands that need structured frontend rendering.

use crate::command_trait::{
    Command, CommandContext, CommandError, CommandMetadata, CommandOutcome, CommandResultDisplay,
};
use crate::commands::registry::{
    CommandContext as LegacyCommandContext, CommandHandler, CommandResult, SharedCommandState,
};
use async_trait::async_trait;
use std::sync::{Arc, Mutex};

/// Adapter that wraps a legacy [`CommandHandler`] as a
/// modern [`Command`].
///
/// Stores the handler + metadata + an optional shared-state
/// handle. The shared handle is forwarded to the legacy
/// context on every `execute`; `None` is acceptable for
/// handlers that don't touch shared state.
pub struct LegacyBuiltinCommand {
    metadata: CommandMetadata,
    handler: Box<dyn CommandHandler>,
    shared: Option<Arc<Mutex<SharedCommandState>>>,
}

impl LegacyBuiltinCommand {
    pub fn new(metadata: CommandMetadata, handler: Box<dyn CommandHandler>) -> Self {
        Self {
            metadata,
            handler,
            shared: None,
        }
    }

    /// Provide a shared-state handle. Required for handlers
    /// that read or write `SharedCommandState` (`/model`,
    /// `/clear`, `/fork`, etc.). Clone-cheap, so the same
    /// handle can be fanned out across many adapters.
    pub fn with_shared(mut self, shared: Arc<Mutex<SharedCommandState>>) -> Self {
        self.shared = Some(shared);
        self
    }
}

#[async_trait]
impl Command for LegacyBuiltinCommand {
    fn metadata(&self) -> &CommandMetadata {
        &self.metadata
    }

    async fn execute(&self, ctx: CommandContext<'_>) -> Result<CommandOutcome, CommandError> {
        // Forward the session cwd from the new `CommandContext`
        // to the legacy one. Codex CR step-5 flagged the
        // earlier `std::env::current_dir()` fallback as a
        // semantic leak: handlers branching on cwd would see
        // the PROCESS cwd, which can disagree with the session
        // cwd (subagent worktrees, SDK-from-stdin, etc.).
        let legacy_ctx = LegacyCommandContext {
            working_directory: ctx.working_directory.to_path_buf(),
            model: ctx.options.main_loop_model.clone(),
            shared: self.shared.clone(),
        };

        // Legacy trait is sync; we call it directly. Tools
        // relying on blocking I/O in command handlers
        // should migrate off the adapter and use the new
        // trait's async `execute` — but today every built-in
        // is in-memory string formatting, so sync-in-async is
        // safe.
        match self.handler.execute(ctx.args, &legacy_ctx) {
            Ok(CommandResult::Message(text)) => Ok(CommandOutcome::Text {
                value: text,
                display: CommandResultDisplay::User,
            }),
            Ok(CommandResult::Action(text)) => Ok(CommandOutcome::Text {
                value: text,
                display: CommandResultDisplay::System,
            }),
            Ok(CommandResult::Error(msg)) => Err(CommandError::Execution(msg)),
            Err(e) => Err(CommandError::Other(e)),
        }
    }
}

/// Construct a metadata for a built-in command in one line.
/// Saves boilerplate when registering dozens of built-ins.
///
/// All built-ins default to: user-invocable, not hidden, not
/// MCP, no availability restriction, `loaded_from: Bundled`.
pub fn builtin_metadata(name: &str, description: &str) -> CommandMetadata {
    let mut m = CommandMetadata::new(name, description);
    m.loaded_from = Some(crate::command_trait::CommandLoadedFrom::Bundled);
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_trait::CommandRegistry;
    use crate::thinking_config::ThinkingConfig;
    use crate::tool_use_context_options::ToolUseContextOptions;
    use anyhow::Result;
    use std::collections::HashMap;

    fn sample_options() -> ToolUseContextOptions {
        ToolUseContextOptions {
            commands: vec![],
            debug: false,
            main_loop_model: "claude-opus-4-7".into(),
            tools: vec![],
            verbose: false,
            thinking_config: ThinkingConfig::Adaptive,
            mcp_clients: vec![],
            mcp_resources: HashMap::new(),
            is_non_interactive_session: true,
            agent_definitions: serde_json::Value::Null,
            max_budget_usd: None,
            custom_system_prompt: None,
            append_system_prompt: None,
            query_source: None,
        }
    }

    struct EchoHandler;
    impl CommandHandler for EchoHandler {
        fn execute(&self, args: &str, _ctx: &LegacyCommandContext) -> Result<CommandResult> {
            Ok(CommandResult::Message(format!("echo: {args}")))
        }
    }

    struct PrintHandler;
    impl CommandHandler for PrintHandler {
        fn execute(&self, _args: &str, ctx: &LegacyCommandContext) -> Result<CommandResult> {
            Ok(CommandResult::Action(format!("using {}", ctx.model)))
        }
    }

    struct FailHandler;
    impl CommandHandler for FailHandler {
        fn execute(&self, _args: &str, _ctx: &LegacyCommandContext) -> Result<CommandResult> {
            Ok(CommandResult::Error("boom".into()))
        }
    }

    struct PanicHandler;
    impl CommandHandler for PanicHandler {
        fn execute(&self, _args: &str, _ctx: &LegacyCommandContext) -> Result<CommandResult> {
            Err(anyhow::anyhow!("ambient failure"))
        }
    }

    #[tokio::test]
    async fn message_result_maps_to_text_user_display() {
        let cmd =
            LegacyBuiltinCommand::new(builtin_metadata("echo", "Echo args"), Box::new(EchoHandler));
        let opts = sample_options();
        let out = cmd
            .execute(CommandContext {
                args: "hello",
                options: &opts,
                working_directory: std::path::Path::new("."),
            })
            .await
            .unwrap();
        match out {
            CommandOutcome::Text { value, display } => {
                assert_eq!(value, "echo: hello");
                assert!(matches!(display, CommandResultDisplay::User));
            }
            other => panic!("expected Text/User, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn action_result_maps_to_text_system_display() {
        let cmd = LegacyBuiltinCommand::new(
            builtin_metadata("print", "Print current model"),
            Box::new(PrintHandler),
        );
        let opts = sample_options();
        let out = cmd
            .execute(CommandContext {
                args: "",
                options: &opts,
                working_directory: std::path::Path::new("."),
            })
            .await
            .unwrap();
        match out {
            CommandOutcome::Text { value, display } => {
                assert_eq!(value, "using claude-opus-4-7");
                assert!(matches!(display, CommandResultDisplay::System));
            }
            other => panic!("expected Text/System, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn error_variant_maps_to_execution_error() {
        let cmd = LegacyBuiltinCommand::new(
            builtin_metadata("fail", "Always fails"),
            Box::new(FailHandler),
        );
        let opts = sample_options();
        let err = cmd
            .execute(CommandContext {
                args: "",
                options: &opts,
                working_directory: std::path::Path::new("."),
            })
            .await
            .unwrap_err();
        match err {
            CommandError::Execution(msg) => assert_eq!(msg, "boom"),
            other => panic!("expected Execution, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn panic_ambient_error_maps_to_other() {
        // anyhow::Error thrown from the legacy handler flows
        // into CommandError::Other via the `#[from]` arm.
        let cmd = LegacyBuiltinCommand::new(
            builtin_metadata("panic", "Thunks into anyhow"),
            Box::new(PanicHandler),
        );
        let opts = sample_options();
        let err = cmd
            .execute(CommandContext {
                args: "",
                options: &opts,
                working_directory: std::path::Path::new("."),
            })
            .await
            .unwrap_err();
        match err {
            CommandError::Other(_) => {}
            other => panic!("expected Other, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn with_shared_forwards_state_handle() {
        // The handle must survive through to the legacy
        // context. Verify by having the handler read the
        // shared handle's `model` field + return it.
        struct ReadSharedHandler;
        impl CommandHandler for ReadSharedHandler {
            fn execute(&self, _: &str, ctx: &LegacyCommandContext) -> Result<CommandResult> {
                let model = ctx
                    .shared
                    .as_ref()
                    .map(|s| s.lock().unwrap().model.clone())
                    .unwrap_or_else(|| "none".into());
                Ok(CommandResult::Action(format!("shared model: {model}")))
            }
        }

        let shared = Arc::new(Mutex::new(SharedCommandState {
            model: "custom-m".into(),
            ..Default::default()
        }));
        let cmd = LegacyBuiltinCommand::new(
            builtin_metadata("readshared", "Reads the shared state"),
            Box::new(ReadSharedHandler),
        )
        .with_shared(shared);

        let opts = sample_options();
        let out = cmd
            .execute(CommandContext {
                args: "",
                options: &opts,
                working_directory: std::path::Path::new("."),
            })
            .await
            .unwrap();
        match out {
            CommandOutcome::Text { value, .. } => {
                assert_eq!(value, "shared model: custom-m");
            }
            _ => panic!("unexpected outcome"),
        }
    }

    #[tokio::test]
    async fn builtin_metadata_bundled_default() {
        let m = builtin_metadata("b", "B");
        assert_eq!(
            m.loaded_from,
            Some(crate::command_trait::CommandLoadedFrom::Bundled)
        );
        assert!(m.user_invocable);
        assert!(!m.is_hidden);
    }

    #[tokio::test]
    async fn registered_in_command_registry() {
        // End-to-end: adapter-wrapped legacy command goes
        // into the new CommandRegistry + dispatches correctly.
        let mut reg = CommandRegistry::new();
        reg.register(Arc::new(LegacyBuiltinCommand::new(
            builtin_metadata("echo", "Echo"),
            Box::new(EchoHandler),
        )));

        let cmd = reg.get("echo").expect("command registered");
        let opts = sample_options();
        let out = cmd
            .execute(CommandContext {
                args: "via registry",
                options: &opts,
                working_directory: std::path::Path::new("."),
            })
            .await
            .unwrap();
        match out {
            CommandOutcome::Text { value, .. } => assert_eq!(value, "echo: via registry"),
            _ => panic!("unexpected"),
        }
    }
}
