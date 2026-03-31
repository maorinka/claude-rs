# claude-rs

A high-performance Rust reimplementation of [Claude Code](https://docs.anthropic.com/en/docs/claude-code), Anthropic's official CLI for Claude.

## What is this?

This is a from-scratch rewrite of Claude Code in Rust, aiming for full feature parity with the original TypeScript/Node.js implementation while delivering dramatically better performance on every client-side metric.

### Architecture

The project is organized as a Cargo workspace with four crates:

| Crate | Purpose |
|-------|---------|
| `claude-core` | API client, SSE streaming, query engine, auth, permissions, config |
| `claude-tools` | Tool registry and implementations (Bash, Read, Write, Edit, Grep, Glob) |
| `claude-tui` | Interactive terminal UI built with `ratatui` + `crossterm` |
| `claude-cli` | Binary entry point, argument parsing, orchestration |

### Features

- Full agentic loop with multi-turn tool use
- Real-time SSE streaming from the Anthropic Messages API
- Interactive TUI with markdown rendering and permission dialogs
- Non-interactive mode for scripting and automation
- OAuth authentication (reads tokens from macOS Keychain, compatible with official client)
- Configurable permission system (bypass / default / interactive-only)
- Extended thinking (reasoning) support

## Building

```bash
cargo build --release
# Binary at: target/release/claude-rs
```

## Usage

```bash
# Interactive TUI mode
./target/release/claude-rs

# Non-interactive (single prompt)
./target/release/claude-rs "explain this codebase"

# With options
./target/release/claude-rs -m claude-sonnet-4-6 --max-turns 5 "fix the bug in main.rs"
```

Requires a valid Anthropic API key (via `ANTHROPIC_API_KEY` env var) or an existing Claude Code OAuth session.

## Status

This project was pretty much built in one shot. It will likely have bugs, rough edges, and missing features. Corrections, fixes, and contributions are welcome.

## Disclaimer

This project is for **educational and research purposes only**. It is not intended for commercial use.

This project is **not affiliated with, endorsed by, or sponsored by Anthropic, PBC**. "Claude" and "Claude Code" are trademarks of Anthropic. Use of these names is solely for identification and interoperability purposes.

This software is provided "AS IS", without warranty of any kind. See [LICENSE](LICENSE) for details.

## Contact

For questions, concerns, or legal inquiries regarding this project: **maor at maor.dev**

## License

MIT License. See [LICENSE](LICENSE).
