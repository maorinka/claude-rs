# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

`claude-rs` is a from-scratch Rust reimplementation of Anthropic's Claude Code CLI, aiming for feature parity with the TypeScript original while improving client-side performance. The binary produced is `target/release/claude-rs` (crate `claude-cli`, bin name `claude-rs`).

Educational/research project, not affiliated with Anthropic.

## Build, Run, Test

```bash
# Build (workspace)
cargo build                      # debug
cargo build --release            # optimized → target/release/claude-rs

# Run a crate's tests
cargo test -p claude-core
cargo test -p claude-tools
cargo test -p claude-tui
cargo test -p claude-cli
cargo test                       # whole workspace

# Run a single test by name (substring match against test fn names)
cargo test -p claude-core --test query_engine_test
cargo test -p claude-tools --test bash_test -- bash_runs_simple_command

# Lint / format
cargo clippy --workspace --all-targets
cargo fmt --all

# Run the binary
cargo run -p claude-cli -- "explain this codebase"
cargo run -p claude-cli -- -m claude-sonnet-4-6 --max-turns 5 "fix the bug"
cargo run -p claude-cli -- --dangerously-skip-permissions "..."
cargo run -p claude-cli -- config     # show resolved paths
```

Tests live in each crate's `tests/` directory (integration-style), not as `#[cfg(test)]` modules. Add new tests as files there.

## Workspace Layout

Four-crate Cargo workspace (resolver = "2"):

| Crate | Role |
|-------|------|
| `claude-core` | API client, SSE streaming, query engine, auth, config, permissions, context |
| `claude-tools` | `ToolExecutor` trait + Bash / Read / Write / Edit / Grep / Glob implementations |
| `claude-tui` | `ratatui` + `crossterm` interactive terminal UI |
| `claude-cli` | Binary entry point (`claude-rs`), CLI parsing, wiring |

Dependency direction is strictly `cli → tui → tools → core` (core depends on nothing internal). Shared deps are pinned in the root `[workspace.dependencies]` — add new shared crates there, not per-crate.

## Architecture: the Agentic Loop

The heart of the system is the **query engine** (`crates/claude-core/src/query/engine.rs`). Understanding it is essential for any non-trivial change.

**Turn lifecycle:** `QueryEngine::run_turn` opens an SSE stream to the Messages API, parses events inline (it does NOT buffer the full response — chunks are line-split and dispatched as they arrive so the TUI can stream text and cancellation is responsive), accumulates content blocks via `ContentBlockAccumulator`, and returns a `TurnResult`:

- `Done(stop_reason)` — terminal
- `ToolUse(Vec<ToolUseInfo>)` — caller must execute tools, feed results back via `add_tool_result`, then call `run_turn` again
- `ContinueRecovery` — `max_tokens` hit; call `run_turn` again (engine has already adjusted state)

**Tool execution lives outside the engine.** Both `claude-cli/src/main.rs` (non-interactive) and `claude-tui/src/app.rs` (TUI) implement their own loop that calls `run_turn`, handles `ToolUse` by looking up the tool in a `ToolRegistry`, runs permission checks, executes, and writes results back. If you add a new transport/mode, replicate this loop — don't try to push tool execution into the engine.

**`max_tokens` recovery** is staged: first response gets an 8k → 64k one-shot escalation (`max_output_tokens_override`), then up to 3 retries where a "resume directly" user message is appended, then terminal. State transitions live in `QueryState` / `TransitionReason` (`query/state.rs`).

**StreamEvent channel** (`types/events.rs`) is how the engine talks to UIs: `TextDelta`, `ThinkingDelta`, `ToolStart`, `ToolResult`, `UsageUpdate`, `Done`, etc. Callers pass an `mpsc::Sender<StreamEvent>` into `run_turn`.

## Auth Resolution

`claude_core::auth::resolve::resolve_auth` picks auth in this priority (matches upstream Claude Code):

1. `ANTHROPIC_API_KEY` env → direct API calls
2. Stored OAuth tokens (macOS Keychain, written by the real `claude` login) → **proxy mode**: we shell out to the installed `claude` binary (`api/claude_proxy.rs`) because OAuth tokens only work with Anthropic's internal SDK
3. `claude` binary on PATH → proxy mode
4. None → friendly "please login" message and exit

When `AuthResolution::OAuthProxy` is returned, `main.rs` skips the query engine entirely and delegates to `stream_via_claude` (non-interactive) or execs `claude` directly (interactive TUI). Keep this fallback path working when touching auth or the CLI entry.

## Tools

Tools implement `ToolExecutor` (`claude-tools/src/registry.rs`): `name`, `input_schema` (JSON schema sent to the model), `call`, plus metadata hooks — `is_read_only`, `is_concurrency_safe`, `is_destructive`, `max_result_size_chars`. The permission layer reads `is_read_only` to decide auto-allow vs. ask.

Register new tools in `claude_tools::build_default_registry`. The registry exposes them to the API via `tool_definitions()`.

## Permissions

`PermissionMode` (`permissions/types.rs`): `Default` (ask for writes), `Bypass` (skip all checks — set by `--dangerously-skip-permissions`), `InteractiveOnly`. `evaluate_permission_sync` returns `Allow | Deny | Ask`. In non-interactive mode the CLI auto-allows `Ask` decisions (the user already committed by passing a prompt); the TUI shows a `PermissionDialog` widget and blocks the turn until the user answers.

## Context / System Prompt

`claude_core::context::system_prompt::build_system_prompt(project_root, tool_descriptions)` assembles the system prompt including environment info (`context/environment.rs`) and git status (`context/git.rs`). Project root detection walks up looking for markers like `.git`, `Cargo.toml`, `package.json`, etc. (`config/paths.rs`).

## Conventions

- **Integration tests only** — no inline `#[cfg(test)] mod tests`. One file per feature in `crates/<crate>/tests/`.
- **Errors**: `anyhow::Result` at call sites, `thiserror` for typed errors that cross module boundaries (see `types/error.rs`, `QueryError`).
- **Async**: tokio with `features = ["full"]`. Long-running operations take a `CancellationToken` (`tokio-util`) and must check it — the engine and tools both honour cancellation mid-stream.
- **Tracing**: use `tracing::{info, warn, debug}`. `--verbose` on the CLI flips the filter from `error` to `debug`.
- **Model default** is `claude-sonnet-4-6` (`api/client.rs` `ApiConfig::default` and `main.rs`). Keep these two in sync if changing.

## Git Workflow

This repo is currently a single-commit project. Develop on the branch specified in the session instructions, commit with descriptive messages, and do not open PRs unless asked.
