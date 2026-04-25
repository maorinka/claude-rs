# Rust Rewrite vs Original TS: Missing / Needs Work

Date: 2026-04-25

Reference code inspected:
- TypeScript: `/Users/maorhadad/projects/claude/claude-code-leaked/src`
- Rust: `/Users/maorhadad/projects/claude/claude-rs/crates`

This file is a current parity checklist. It supersedes the stale parts of
`feature-gap-analysis.md`: several items previously marked missing now exist,
but many are partial, skeletal, or incorrectly wired.

## Summary

The Rust rewrite has a substantial amount of surface area implemented: core
API streaming, a TUI, command registry, many tools, MCP client pieces, memory
modules, hooks types, migrations, bridge basics, and Rust-only additions such
as Teams/Sandbox/VCR/File History.

The largest remaining gaps are behavioral depth, not just missing files:

- Tool exposure/gating does not match TS in several cases.
- MCP resource tools are registered but stubbed.
- Slash commands are broad but many are manual/link-only replacements for TS
  interactive flows.
- Query execution is much thinner than TS around context, compaction, hooks,
  tool-result handling, analytics, and permission flow.
- Bridge/direct-connect/upstream proxy are far from TS parity.
- UI is a small ratatui subset of the original Ink UI.
- Migrations, remote services, analytics, and hosted/managed account behavior
  are partial.

## P0: Wrong Or Risky Behavior

### Tool registry exposes tools TS keeps gated

Rust registers several tools unconditionally in
`crates/claude-tools/src/lib.rs`, including:

- Task v2 tools: `TaskCreateTool`, `TaskListTool`, `TaskUpdateTool`,
  `TaskGetTool`, `TaskOutputTool`
- `LSPTool`
- `ToolSearchTool`
- Team tools: `TeamCreateTool`, `TeamDeleteTool`
- Worktree tools: `EnterWorktreeTool`, `ExitWorktreeTool`
- `PowerShellTool`
- `SleepTool`
- MCP auth/resources tools

TS gates comparable tools in `src/tools.ts` with checks such as:

- `isTodoV2Enabled()`
- `ENABLE_LSP_TOOL`
- `isToolSearchEnabledOptimistic()`
- `isWorktreeModeEnabled()`
- `isAgentSwarmsEnabled()`
- `PROACTIVE` / `KAIROS`
- `isPowerShellToolEnabled()`
- `USER_TYPE === 'ant'`
- REPL mode filtering and `REPL_ONLY_TOOLS`
- deny-rule filtering via `filterToolsByDenyRules`

Needs work:
- Align Rust `build_default_registry()` with TS `getAllBaseTools()` and
  `getTools()`.
- Add deny-rule filtering before tools are exposed to the model.
- Hide REPL-only primitive tools when REPL mode is active.
- Keep tool ordering stable for prompt-cache compatibility.
- Revisit whether `SleepTool`, task v2, teams, worktree, LSP, ToolSearch, and
  PowerShell should be default-visible.

### MCP resource tools need deeper parity

Files:
- `crates/claude-tools/src/mcp_resource_tools.rs`
- TS reference: `src/tools/ListMcpResourcesTool`,
  `src/tools/ReadMcpResourceTool`, `src/services/mcp`

Improved:
- Rust now registers manager-backed `ListMcpResourcesTool` and
  `ReadMcpResourceTool` in CLI startup.
- `ListMcpResourcesTool` reads live resources from `McpManager` and supports
  server filtering.
- `ReadMcpResourceTool` reads from the requested connected MCP server and
  returns manager errors when the server/resource is unavailable.

Still needs work:
- Verify pagination/cursors if applicable, resource templates, and
  server-not-found error text against TS.
- Include MCP resource output in the same result shape the model expects.
- Add integration tests with a real or fake MCP server exposing resources.

### Tool executor is simplified

Files:
- `crates/claude-core/src/query/tool_executor.rs`
- TS reference: `src/services/tools/toolExecution.ts`,
  `src/query.ts`, `src/QueryEngine.ts`

Needs work:
- Track concurrent vs exclusive tool counts correctly.
- Stream tool progress/results with TS-equivalent events.
- Apply tool-result budget/truncation before adding results to history.
- Dispatch PreToolUse/PostToolUse/PostToolUseFailure hooks.
- Run permission request flow from the executor rather than relying on thin
  checks in individual tools.
- Normalize/cancel tool results exactly like TS, including synthetic cancel
  messages and partial tool-use repair.
- Add tests for mixed concurrent/exclusive scheduling.

### Query user context is date-only

Files:
- `crates/claude-core/src/query/engine.rs`
- `crates/claude-core/src/context/system_prompt.rs`
- TS reference: `src/context.ts`, `src/utils/api.ts`,
  `src/utils/claudemd.ts`, `src/memdir/*`

Rust prepends a `<system-reminder>` with only `currentDate` during query.
TS builds richer cached user/system context, including CLAUDE.md aggregation,
memory files, git status, injected extra directories, cache-breaker injection,
and bare/disable flags.

Needs work:
- Reuse Rust context builders in the actual query prepend path.
- Match TS behavior for `CLAUDE_CODE_DISABLE_CLAUDE_MDS`, bare mode, and
  additional directories.
- Cache context at the same lifecycle points as TS.
- Ensure git status appears where TS places it and with the same truncation
  behavior.

## P0: Missing Core TS Behavior

### Query engine features

Files:
- `crates/claude-core/src/query/engine.rs`
- `crates/claude-core/src/query/state.rs`
- TS reference: `src/query.ts`, `src/QueryEngine.ts`

Missing or partial:
- Microcompact with cache-edit of pending content.
- Context collapse flow.
- Attachment message prefetch.
- Relevant-memory prefetch.
- StopFailure hook execution.
- Tool-use summarization.
- Structured-output enforcement.
- Full max-turn and compaction edge-case parity.
- File-history snapshot integration during query.
- Detailed analytics events for query lifecycle.
- Full permission request/deny/ask flow while tools execute.
- Retry/recovery behavior for every API error class TS handles.

Needs work:
- Build a TS/Rust behavior matrix for each query transition.
- Add transcript-level tests for tool-use, cancel, compact, max-token recovery,
  stop hooks, and resumed sessions.

### Bridge / direct-connect / upstream proxy

Rust files:
- `crates/claude-core/src/bridge/*`
- `crates/claude-core/src/proxy/*`
- `crates/claude-core/src/remote/*`

TS reference:
- `src/bridge/*`
- `src/server/directConnectManager.ts`
- `src/server/createDirectConnectSession.ts`
- `src/upstreamproxy/upstreamproxy.ts`
- `src/upstreamproxy/relay.ts`

Missing or partial:
- Full bridge messaging.
- `initReplBridge` / `replBridge` behavior.
- `remoteBridgeCore` parity.
- Session runner integration.
- Bridge permission callbacks.
- Trusted-device flow.
- Work secret lifecycle.
- Capacity wake.
- Flush gate.
- Poll config.
- Inbound attachments/messages.
- Bridge UI.
- Direct WebSocket session manager.
- Upstream relay/proxy.

Needs work:
- Decide whether Rust aims to support the same bridge/server product surface.
- If yes, port this as a subsystem, not as isolated helpers.

### MCP feature depth

Rust has a real MCP client/manager, but TS still has broader behavior.

Missing or partial:
- WebSocket transport.
- In-process transport.
- OAuth/XAA IdP command parity.
- Official registry integration.
- Elicitation dialog/UI integration.
- Parsing warnings UI.
- Reconnect/fallback UX.
- MCP output storage hooked into tool results.
- MCP instructions delta fully integrated into query/system prompt lifecycle.
- Skill builders for MCP.
- Channel allowlist/permissions/notifications parity.
- VS Code SDK MCP integration.

Needs work:
- Finish resource tools first because they are visible and currently stubbed.
- Then wire auth, elicitation, reconnect, and UI flows.

## P1: Tools Needing Further Work

### BashTool

Files:
- `crates/claude-tools/src/bash.rs`
- `crates/claude-tools/src/bash_security.rs`
- `crates/claude-tools/src/bash_commands.rs`
- `crates/claude-tools/src/read_only_validation.rs`
- `crates/claude-tools/src/sed_validation.rs`
- TS reference: `src/tools/BashTool/*`

Needs work:
- Full per-command permission matrix parity.
- Stronger read-only validation parity.
- Sed parser/validator parity.
- Git operation tracking.
- Background task spawning and foreground registration.
- Chunked streaming output parity.
- Cwd reset if outside project.
- File-history tracking for shell-modified files.
- Better command semantics for pipelines, shell builtins, and platform quirks.

### FileEditTool

Files:
- `crates/claude-tools/src/edit.rs`
- `crates/claude-tools/src/edit_quote_style.rs`
- TS reference: `src/tools/FileEditTool/*`

Needs work:
- Exact TS diff output shape.
- Original-file capture in file history.
- CRLF preservation checks.
- LSP diagnostic clearing after edit.
- Settings-file validation.
- Conditional skill activation by edited path.
- Multi-edit behavior and error messages parity.
- More tests for stale read state, partial reads, repeated replacements,
  newline handling, and quote-style preservation.

### FileWriteTool

Files:
- `crates/claude-tools/src/write.rs`
- TS reference: `src/tools/FileWriteTool/*`

Needs work:
- Permission validation parity.
- File-history integration.
- Git diff output.
- Append mode if TS supports it in the active reference path.
- More precise stale-write behavior.

### WebFetchTool

Files:
- `crates/claude-tools/src/web_fetch.rs`
- `crates/claude-tools/src/web_fetch_preapproved.rs`
- TS reference: `src/tools/WebFetchTool/*`

Improved since the stale report:
- Rust now accepts a `prompt` parameter and can call a secondary model.

Still needs work:
- URL scheme validation.
- Domain permission gating.
- Preapproved-host behavior parity.
- HTML to Markdown conversion parity. Rust currently strips HTML manually.
- Redirect, content-type, encoding, and size-limit behavior parity.
- Copyright/quote-limit behavior exactly matching TS.
- Better fallback when no secondary model is registered.

### SkillTool

Files:
- `crates/claude-tools/src/skill_tool.rs`
- `crates/claude-tools/src/bundled_skills/*`
- Rust plugins: `crates/claude-core/src/plugins/*`
- TS reference: `src/tools/SkillTool/*`, `src/skills/*`

Needs work:
- Dynamic skill discovery parity.
- Conditional activation by path/content.
- Dynamic skill directory additions.
- MCP-based skill execution.
- Plugin-provided skill loading parity.
- Skill search/index behavior parity.

### AgentTool / subagents

Files:
- `crates/claude-tools/src/agent_tool.rs`
- `crates/claude-tools/src/agents/*`
- Rust teams: `crates/claude-core/src/teams/*`
- TS reference: `src/tools/AgentTool/*`

Needs work:
- Fork-mode parity.
- Progress/event reporting parity.
- Permission propagation and deny-list filtering.
- Agent transcript/session persistence.
- Coordinator/worker prompt and allowed-tool parity.
- Background agent lifecycle parity.

### Task tools / Todo tools

Files:
- `crates/claude-tools/src/task_tools.rs`
- `crates/claude-tools/src/todo_write.rs`
- TS reference: `src/tools/TaskCreateTool/*`,
  `src/tools/TodoWriteTool/*`, `src/utils/tasks.ts`

Needs work:
- Respect TS `isTodoV2Enabled()` gating.
- Persistent task store parity.
- Owner/team awareness.
- `blocks` / `blockedBy`.
- `TaskCreated` and `TaskCompleted` hooks.
- Active spinner verb / active form behavior.
- Auto-classifier input integration.
- Cross-session task behavior.

### MCPAuthTool

Files:
- `crates/claude-tools/src/mcp_auth_tool.rs`
- `crates/claude-core/src/mcp/auth_*`
- TS reference: `src/services/mcp/auth.ts`, `src/commands/mcp/xaaIdpCommand.ts`

Needs work:
- End-to-end OAuth parity.
- Auth cache behavior.
- XAA IdP flows.
- Error messages and recovery prompts matching TS.

### Worktree tools

Files:
- `crates/claude-tools/src/worktree_tools.rs`
- TS reference: worktree mode helpers and UI flows.

Needs work:
- Gate with TS `isWorktreeModeEnabled()`.
- Worktree create/remove hooks.
- UI exit dialog parity.
- Session cwd restore edge cases.
- Permissions/path checks for nested vs sibling worktrees.

### PowerShellTool

Files:
- `crates/claude-tools/src/powershell.rs`
- TS reference: `src/tools/PowerShellTool/*`,
  `src/utils/shell/shellToolUtils.ts`

Needs work:
- Gate with `isPowerShellToolEnabled()`.
- Confirm Windows-specific behavior.
- Permission and read-only semantics parity with Bash.

### ToolSearchTool

Files:
- `crates/claude-tools/src/tool_search.rs`
- TS reference: `src/utils/toolSearch.ts`, `src/tools/ToolSearchTool/*`

Needs work:
- Gate optimistically the same way TS does.
- Integrate request-time tool deferral threshold.
- Include MCP tools in counting/search where TS does.

## P1: Slash Commands

Files:
- `crates/claude-core/src/commands/builtin.rs`
- `crates/claude-core/src/commands/registry.rs`
- TS reference: `src/commands/*`, `src/commands.ts`

Broad command coverage now exists, but many commands are not equivalent.

Commands that appear especially partial or manual:
- `/feedback`: local transcript text instead of TS submission/form flow.
- `/upgrade`: static links instead of TS flow.
- `/install-github-app`: manual steps instead of interactive GitHub wizard.
- `/install-slack-app`: static instructions.
- `/chrome`: static docs/store pointers.
- `/desktop`: static download/docs instead of session handoff.
- `/mobile`: static instructions.
- `/terminalSetup`: diagnostics/instructions only.
- `/heapdump`: informational only.
- `/remote-env`: local/env display, not full TS remote integration.
- `/remote-setup`: setup instructions, not full flow.
- `/passes`: likely prompt-only approximation.
- `/mcp`: management surface is far thinner than TS.
- `/plugin`: management surface is far thinner than TS.
- `/skills`: listing only vs full TS UI.
- `/agents`: no full running-agent management.
- `/resume`: needs session-search/selection parity.
- `/copy`: depends on shell clipboard tools, not full TS UI behavior.
- `/export`: verify format/session-content parity.
- `/memory`: verify against TS memory command and memdir behavior.
- `/permissions`: verify against full TS permissions UI.
- `/doctor`: verify against TS doctor checks.
- `/context`: verify against TS context visualization/noninteractive output.
- `/output-style`: currently deprecated/config-style behavior; TS has richer UI.

Needs work:
- For every command, classify as `full`, `partial`, `prompt-only`,
  `link-only`, or `missing`.
- Add golden tests against TS command output where possible.
- Wire commands to live TUI dialogs rather than static text for interactive
  flows.

## P1: Hooks

Files:
- `crates/claude-core/src/hooks/*`
- TS reference: `src/hooks/*`, `src/query/stopHooks.ts`,
  hook dispatch sites across tools/query/commands.

Rust has hook types and runner pieces, but dispatch is incomplete.

Needs work:
- Fire `PreToolUse`, `PostToolUse`, and `PostToolUseFailure` from the tool
  executor.
- Fire `Stop` and `StopFailure` from the query engine.
- Fire `TaskCreated` / `TaskCompleted` from task tools.
- Fire session, compact, config, cwd, file, worktree, teammate, permission,
  and elicitation hooks at the same sites as TS.
- Implement aggregation/blocking behavior exactly.
- Ensure hook outputs are inserted into transcript/context like TS.

## P1: Memory / Context

Files:
- `crates/claude-core/src/memdir/*`
- `crates/claude-core/src/session_memory.rs`
- `crates/claude-core/src/context/*`
- TS reference: `src/memdir/*`, `src/services/SessionMemory/*`,
  `src/utils/claudemd.ts`, `src/context.ts`

Rust now has memdir modules, but integration is incomplete.

Needs work:
- Full MEMORY.md discovery and loading.
- Relevant-memory search/prefetch during query.
- Auto-memory detection and writeback parity.
- Team memory paths and sync parity.
- Session memory compaction.
- CLAUDE.md aggregation behavior parity, including imports/injections,
  additional dirs, truncation, and cache lifecycle.
- Memory command UI parity.

## P1: Compaction

Files:
- `crates/claude-core/src/compact/*`
- `crates/claude-core/src/query/engine.rs`
- TS reference: `src/services/compact/*`, `src/query.ts`

Needs work:
- Microcompact.
- API microcompact.
- Time-based microcompact config.
- Session-memory compact.
- Post-compact cleanup.
- Compact warning hook.
- Reactive compact details and transcript markers.
- Compaction UI parity.

## P1: Permissions / Security

Files:
- `crates/claude-core/src/permissions/*`
- `crates/claude-tools/src/bash_security.rs`
- `crates/claude-tools/src/read_only_validation.rs`
- TS reference: `src/utils/permissions/*`, tool permission code,
  Yolo/auto-mode classifier.

Needs work:
- Full permission context matching TS.
- Deny-rule filtering before model exposure.
- Permission request UI flow parity.
- Auto-mode classifier integration.
- Filesystem permission evaluator parity.
- Shell command permission matrix parity.
- Project/trust dialog lifecycle parity.
- Sandbox prompt and execution parity across platforms.

## P1: API / Services

Files:
- `crates/claude-core/src/api/*`
- `crates/claude-core/src/auth/*`
- `crates/claude-core/src/remote/*`
- TS reference: `src/services/api/*`, `src/services/*`

Missing or partial:
- Files API.
- Grove/session ingress.
- Ultrareview quota/overage credit.
- First-token-date logic.
- Referral/admin request helpers.
- Bootstrap service calls.
- Metrics opt-out plumbing.
- Prompt cache break detection.
- Empty usage helpers.
- Detailed logging/diagnostics depth.
- Full remote session manager behavior.
- Remote managed settings.
- Settings sync.
- Policy limits.
- Prompt suggestion service.
- Agent summary service.
- Tool-use summary service.
- MagicDocs.
- AutoDream.
- ExtractMemories.
- Tips.

Needs work:
- Decide which cloud/backend product surfaces are in scope for Rust.
- For in-scope pieces, port by service boundary rather than isolated helpers.

## P1: Analytics / Diagnostics

Files:
- `crates/claude-core/src/analytics/*`
- `crates/claude-core/src/diag_logs.rs`
- TS reference: `src/services/analytics/*`, `src/utils/diagLogs.ts`

Needs work:
- Match TS event taxonomy.
- Add query/tool/command/hook/permission/session lifecycle events.
- Add no-PII logging behavior parity.
- Add cost/usage details parity.
- Ensure diagnostics are available in command/UI flows.

## P1: Migrations

Files:
- `crates/claude-core/src/migrations/*`
- TS reference: `src/migrations/*`

Rust currently runs a subset:
- `migrateReplBridgeEnabledToRemoteControlAtStartup`
- `migrateFennecToOpus`
- `migrateLegacyOpusToCurrent`
- `migrateSonnet45ToSonnet46`
- `migrateBypassPermissionsAcceptedToSettings`
- `migrateSonnet1mToSonnet45`

Needs work:
- Port or explicitly mark out-of-scope the remaining TS migrations:
  - auto-updates settings migration
  - enable-all-project-MCP-servers migration
  - opus-to-opus1m migration if still relevant
  - auto-mode opt-in reset
  - pro-to-opus default reset
  - any subscriber/feature-gated migrations skipped due to missing state
- Confirm migrations run at CLI startup and are persisted.
- Add fixture tests using real TS config examples.

## P1: UI / TUI

Files:
- `crates/claude-tui/src/*`
- TS reference: `src/components/*`, `src/screens/*`, `src/ink/*`,
  `src/dialogLaunchers.tsx`

Rust has a smaller ratatui UI. Missing or partial:
- MCP server approval, reconnect, parsing warnings, server list/detail,
  settings, capabilities, elicitation dialogs.
- IDE/Chrome/Desktop onboarding.
- Bridge dialogs.
- Export dialog.
- Worktree exit dialog.
- Idle return dialog.
- Auto-mode opt-in dialog.
- Resume conversation/session picker UI.
- Invalid config/settings dialogs.
- Cost threshold dialog.
- Language picker.
- Context suggestions/visualization.
- Doctor screen/system health panel.
- LSP status UI.
- Global search/quick-open/history search.
- Rich file-edit diff and updated-message rendering.
- Dialog launcher infrastructure.
- Interactive helpers parity.
- Full Ink/Yoga layout features are intentionally not present, but equivalent
  UX still needs product decisions.

Needs work:
- Decide whether Rust TUI should match TS UI feature-for-feature or provide a
  lean equivalent.
- For product parity, port workflows first, visuals second.

## P2: Types / Schemas / Constants / Prompts

Files:
- `crates/claude-core/src/types/*`
- `crates/claude-core/src/constants/*`
- `crates/claude-core/src/prompts/*`
- TS reference: `src/types/*`, `src/schemas/*`, `src/constants/*`,
  `src/constants/prompts.ts`

Needs work:
- Hook schemas parity.
- SDK/control schemas parity.
- Agent SDK type parity.
- Sandbox type parity.
- Full constants parity, especially prompts and output styles.
- Cyber risk, system prompt sections, managed env, model aliases, betas, API
  limits, tool limits, display constants all need periodic sync.
- Add tests that pin source-of-truth lists where TS has closed sets.

## P2: CLI / Entrypoints

Files:
- `crates/claude-cli/src/main.rs`
- TS reference: `src/entrypoints/*`, `src/main.tsx`, `src/setup.ts`,
  `src/replLauncher.tsx`

Needs work:
- Full startup orchestration parity.
- SDK entrypoint parity.
- MCP entrypoint parity.
- Init/setup sequencing parity.
- Preflight checks parity.
- Graceful shutdown behavior parity.
- Noninteractive/headless output shape parity.
- CLI flags and env var compatibility audit.

## P2: Native / Utility Gaps

TS has several utility/native-adjacent modules with no or partial Rust
equivalent:

- color diff logic
- file index / fast search helpers
- PDF utilities
- image paste/validation edge cases
- asciicast helpers
- teleport/session sharing helpers
- cross-project resume
- IDE path conversion
- release notes updater
- process ancestry helpers beyond current minimal ports
- CA certificates / mTLS config parity
- terminal querying/focus/hyperlink support parity

Needs work:
- Classify each as product-required, CLI-required, or TS UI-only.

## Rust-Only Or Divergent Subsystems To Reconcile

Rust includes subsystems that do not map cleanly to the TS reference or are
deeper than the TS version:

- Teams / coordinator modules
- Sandbox executor
- LSP client/manager depth
- VCR recorder/player
- Workflows
- File history tracker
- Some compact internals

Needs work:
- Decide whether these are intended product additions or temporary porting
  scaffolding.
- Ensure they do not expose tools or prompt sections when TS would not.
- Add compatibility notes for any intentional divergence.

## Suggested Next Order

1. Fix tool registry gating and deny-rule filtering.
2. Wire MCP resource tools to `McpManager`.
3. Audit slash commands into `full/partial/prompt-only/link-only/missing`.
4. Move rich user/system context into query execution.
5. Complete hook dispatch from tool/query/task flows.
6. Add transcript-level tests for query/tool/cancel/compact/resume behavior.
7. Decide scope for bridge/direct-connect/upstream proxy before porting more
   isolated helpers.
8. Update or replace `feature-gap-analysis.md` once this checklist is validated.
