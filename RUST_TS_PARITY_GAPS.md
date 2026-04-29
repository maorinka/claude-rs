# Rust Rewrite vs Original TS: Missing / Needs Work

Date: 2026-04-28

Latest live proxy comparison: 2026-04-28

Reference code inspected:
- TypeScript source snapshot: `/Users/maorhadad/Downloads/src`
- Installed TypeScript CLI: `claude` v2.1.121
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
- MCP resource tools are manager-backed and covered against a fake stdio
  resource server; remaining work is auth/elicitation/reconnect/UI depth.
- Slash commands are broad but many are manual/link-only replacements for TS
  interactive flows.
- Query execution is much thinner than TS around context, compaction, hooks,
  tool-result handling, analytics, and permission flow.
- Bridge/direct-connect/upstream proxy are far from TS parity.
- UI is a small ratatui subset of the original Ink UI.
- Migrations, remote services, analytics, and hosted/managed account behavior
  are partial.

## Current Live Context Diff

Captured through `scripts/run_parity_capture.py` on 2026-04-28:

- Capture directory:
  `/tmp/claude-rs-parity-toolsearch-request-fixed`

Now matching:

- `model`, `max_tokens`, `stream`, `thinking`, and `context_management`.
- `output_config` effort now uses TS-supported values only; stale `xhigh`
  settings/env values are coerced before reaching the API.
- Message shape.
- Prompt cache marker count.
- Tool count: 87 vs 87.
- Tool names, tool order, and tool schemas. This now includes the installed
  TS 2.1.121 Explore-agent description and current MCP connector schemas in
  the embedded/live tool contracts.
- Request-time ToolSearch defaults now follow TS for the live debug-proxy
  path: with `ENABLE_TOOL_SEARCH` unset and a first-party provider pointed at
  a non-first-party base URL, both TS and Rust strip `ToolSearch`, send MCP
  tools inline, omit `defer_loading`, and omit the tool-search beta header.
- Claude.ai MCP servers now match the TS auth flow for Cloudflare: a live
  proxy 401 is classified as `needs-auth`, the two auth shadow tools are
  exposed, and the MCP needs-auth cache short-circuits future HTTP/SSE
  connection attempts.
- MCP instruction block ordering after sorting connected instruction deltas by
  server name.
- Stream-json event sequence and event key shapes.
- `system/init` scalar keys.
- Assistant message and usage payload shapes.
- Final result, usage, iteration, and model-usage payload shapes.
- Static environment/context-management prompt formatting, including the
  installed TS `# Context management` heading and environment bullet spacing.
- Environment model display and knowledge-cutoff text now follow the installed
  TS `getMarketingNameForModel` / `getKnowledgeCutoff` behavior for the Opus
  4.6 1M path.
- Two-turn dynamic skill discovery after `Read` now follows TS shape:
  discovered skill listings are folded into the adjacent `tool_result` content
  before the next API request rather than emitted as a separate user turn.
- `Read` line-number output now preserves TS trailing-newline behavior
  (`content.split(/\r?\n/)`), including the final numbered blank line.
- `Read` offset handling now follows TS 1-based semantics, and repeated
  same-file/same-range reads now return the TS `file_unchanged` stub when the
  file mtime is unchanged instead of re-sending duplicate file content.
- `Read` tool result metadata now keeps raw file content like TS
  `FileReadTool`, and model-facing tool-result text is line-numbered only
  when the CLI maps tool data into the next model request.
- `Read`, `Edit`, and `Write` now expand relative paths against the
  request-scoped cwd before permission checks and filesystem access, matching the TS
  `backfillObservableInput` / `expandPath` flow.
- `Read` now blocks the same stdio/device aliases as TS, including
  `/dev/full`, `/dev/stdout`, `/dev/stderr`, `/dev/fd/{0,1,2}`, and
  `/proc/.../fd/{0,1,2}`.
- `Read` now uses TS's macOS screenshot fallback that swaps regular space and
  narrow no-break space before `AM`/`PM` when the requested screenshot path is
  missing.
- Installed CLI identity is synced to the current TS CLI version observed in
  live captures: `2.1.121`, including the OAuth billing version string.
- Stream-json assistant events for multi-block responses now split per content
  block like TS, instead of emitting one combined thinking+tool_use message.
- Stream-json tool-use assistant events now preserve the real API message id,
  model, and usage captured from the SSE stream instead of synthesizing a local
  message envelope.
- `--include-partial-messages` and `CLAUDE_CODE_INCLUDE_PARTIAL_MESSAGES` now
  follow TS stream-json flow: raw SSE records are emitted as `stream_event`
  only when enabled, ping events are suppressed, the completed assistant block
  appears before `content_block_stop`, `message_stop` is preserved, and the SDK
  `system/status` requesting event is emitted before the request stream.
- Hook lifecycle stream-json events now go through a TS-style process-wide
  hook event bus with the same default allowlist (`SessionStart`, `Setup`) and
  all-events switch for `--include-hook-events` / `CLAUDE_CODE_REMOTE`.
- Print-mode stream-json now emits `UserPromptSubmit` hook events before
  `system/init`, matching TS startup flow, and tool hook events use the SDK
  `hook_started` / `hook_progress` / `hook_response` system-event shape,
  including progress-interval streaming for long-running command hooks.
- Hook settings merging now appends hook event arrays across settings and
  enabled plugin hook files instead of overwriting earlier arrays, so user
  hooks and plugin hooks both run. The `run pwd --include-hook-events` smoke
  now matches TS on three Stop hook starts and three Stop hook responses.
- Rust stream-json no longer synthesizes `rate_limit_event` from
  `hook_started`; the remaining timing gap is tracked as API status-listener
  plumbing instead of hook-event coupling.
- Latest normal stream-json smoke:
  `/tmp/claude-rs-parity-stream-normal-after-partial`.
- Latest partial-message stream-json smoke:
  `/tmp/claude-rs-parity-partial-messages-2`.
- Latest hook-event stream-json smoke:
  `/tmp/claude-rs-parity-rate-limit-order-2`.
- Text `--print` mode now buffers assistant text and prints only successful
  final output like TS, so `--max-turns` errors do not leak partial assistant
  text before `Error: Reached max turns (...)`; the text-mode max-turns error
  is emitted on stdout like TS `claude -p`.
- `remote-control` / `rc` is now hidden from root help like the TS Commander
  registration, accepts the installed TS bridge flags (`--name`,
  `--remote-control-session-name-prefix`, `--permission-mode`, `--debug-file`,
  `--spawn`, `--capacity`, `--[no-]create-session-in-dir`), prints the
  TS-style custom help screen before normal CLI parsing, and applies the TS
  `--spawn` / `--capacity` validation rules.

Expected dynamic differences:

- `metadata.user_id.session_id` changes per process.
- `x-anthropic-billing-header` contains a per-run `cch` value.
- System context recent-commit text changes as commits are made.
- System context git status changes while the parity run itself is executed
  from a dirty worktree.

Remaining first-turn prompt-context difference:

- The available-skills block has the same entries, but plugin command/skill
  order can differ between runs.
- Latest capture reports `equal ignoring skill order: yes`; the first-turn
  API body is now equal after normalizing the plugin skill/command order that
  TS itself emits nondeterministically.

### TS Skill And Command Ordering Rules

The TS top-level command source order is explicit in
`src/commands.ts::loadAllCommands`:

1. Bundled skills.
2. Built-in plugin skills.
3. `.claude/skills` and legacy `.claude/commands` skills.
4. Workflow commands.
5. Plugin commands.
6. Plugin skills.
7. Built-in slash commands.

Within regular skill directories, TS preserves the `fs.readdir` entry array
order because it maps entries through `Promise.all` and uses the returned array.

Within plugin command and plugin skill directories, TS does not apply a stable
sort. The relevant functions are:

- `src/utils/plugins/walkPluginMarkdown.ts::walkPluginMarkdown`
- `src/utils/plugins/loadPluginCommands.ts::collectMarkdownFiles`
- `src/utils/plugins/loadPluginCommands.ts::loadSkillsFromDirectory`

Those functions call `Promise.all(entries.map(...))`, but push loaded files into
a shared array inside each async callback. That means plugin file order is
completion order, not alphabetic order and not manifest order. Multiple TS
captures in this repo already show different `superpowers:*` orders between
runs. Rust should therefore mirror the same source buckets and concurrent
loading behavior, not force a fixed hardcoded order for private plugins.

## P0: Wrong Or Risky Behavior

### Tool registry exposure/gating still needs request-time parity

Rust registers several tools unconditionally in
`crates/claude-tools/src/lib.rs`, including:

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

Improved:
- Rust now gates `LSPTool`, `SleepTool`, team tools, and `PowerShellTool`
  at registry construction.
- Rust now matches TS Task v2/TodoWrite visibility for interactive vs
  noninteractive startup, including `CLAUDE_CODE_ENABLE_TASKS`.
- Rust now keeps ToolSearch out of the registry when explicitly disabled by
  `ENABLE_TOOL_SEARCH` or the experimental-beta kill switch.
- CLI startup now filters blanket-denied tools before exposing them to the
  model, including the refreshed ToolSearch snapshot.
- Rust now applies the TS request-time ToolSearch gate in the API layer:
  model support, explicit/env mode, non-first-party proxy default-disable,
  `auto:N` token-count threshold with TS's character fallback when the
  count-tokens call is unavailable, discovered `tool_reference` scan, deferred
  MCP and `shouldDefer` built-in tool filtering, and beta-header insertion only
  when ToolSearch/defer-loading is actually used.
- Rust now also reads TS compact-boundary
  `compactMetadata.preCompactDiscoveredTools`, so deferred tools loaded before
  compaction remain available afterward without hardcoding private tool names.
- Worktree tools remain visible by default because the inspected TS reference
  now returns `true` from `isWorktreeModeEnabled()`.
- REPL mode now hides the same primitive tools as TS when the `REPL` tool is
  active: `Read`, `Write`, `Edit`, `Glob`, `Grep`, `Bash`,
  `NotebookEdit`, and `Agent`.

Needs work:
- Keep tool ordering stable for prompt-cache compatibility.
- Broaden deny-rule filtering to all registry construction paths and add MCP
  integration coverage with real connected tools.
- Revisit MCP auth/resource tool visibility against TS `specialTools` and MCP
  connection state.

### MCP resource tools need deeper parity

Files:
- `crates/claude-tools/src/mcp_resource_tools.rs`
- TS reference: `src/tools/ListMcpResourcesTool`,
  `src/tools/ReadMcpResourceTool`, `src/services/mcp`

Improved:
- Rust now registers manager-backed `ListMcpResourcesTool` and
  `ReadMcpResourceTool` in CLI startup using the TS capability rule: add them
  once, only when a connected MCP server advertises `resources`.
- `ListMcpResourcesTool` reads live resources from `McpManager` and supports
  server filtering.
- `ReadMcpResourceTool` reads from the requested connected MCP server and
  returns manager errors when the server/resource is unavailable.
- `ListMcpResourcesTool` now follows the TS output contract for tool data:
  an array of resources with `mimeType` casing, plus the TS empty-list
  tool-result text when mapped back into model context.
- `ReadMcpResourceTool` now follows TS binary-resource handling: text
  resources stay inline, but blob contents are decoded, written to the
  session `tool-results` directory, and replaced with `blobSavedTo` plus the
  TS saved-binary message instead of sending base64 into model context.
- `ListMcpResourcesTool` and `ReadMcpResourceTool` now use the TS explicit
  missing-server error shape (`Server "..." not found. Available servers: ...`)
  instead of silently returning an empty resource list or a Rust manager error.
- `ReadMcpResourceTool` now checks the connected server's MCP capabilities and
  returns the TS-style `Server "..." does not support resources` error instead
  of attempting `resources/read` against servers without resource support.
- `ListMcpResourcesTool` now matches TS for a named but disconnected/failed MCP
  server: the server is considered found, but contributes no resources.
- `ReadMcpResourceTool` now matches TS for a named but disconnected/failed MCP
  server by returning `Server "..." is not connected`.
- Resource-tool binary persistence now uses a shared Rust `tool-results`
  storage helper instead of a one-off MCP helper, matching the TS
  `toolResultStorage` / `mcpOutputStorage` split more closely.
- TS does not call MCP resource-template APIs or manually walk paginated
  resource cursors in `fetchResourcesForClient`; it calls SDK
  `resources/list` once and adds the `server` field. Rust follows that same
  flow.
- Resource list/read now has integration coverage with a fake stdio MCP server
  exposing `resources/list` and `resources/read`.

### Tool executor is simplified

Files:
- `crates/claude-core/src/query/tool_executor.rs`
- TS reference: `src/services/tools/toolExecution.ts`,
  `src/query.ts`, `src/QueryEngine.ts`

Needs work:
- Stream tool progress/results with TS-equivalent events.
- Move the duplicated CLI/TUI PreToolUse/PostToolUse/PostToolUseFailure hook
  and permission flow into the shared executor so there is one TS-equivalent
  execution path.
- Normalize/cancel tool results exactly like TS, including synthetic cancel
  messages and partial tool-use repair.

Improved:
- Rust CLI and TUI now both dispatch PreToolUse, PostToolUse, and
  PostToolUseFailure hooks around tool execution.
- Rust now reads `CLAUDE_CODE_MAX_TOOL_USE_CONCURRENCY` and caps concurrent
  tool execution the same way TS `runToolsConcurrently(... all(...))` does,
  defaulting to 10.
- Existing executor tests cover mixed concurrent/exclusive scheduling and
  ordered result yielding; a regression test now covers the TS concurrency cap.
- The shared executor now mirrors TS Bash-error cancellation: a failed Bash
  call cancels parallel siblings and yields ordered synthetic cancellation
  results instead of letting sibling tools continue silently. Synthetic
  executor errors now use TS `<tool_use_error>...</tool_use_error>` model
  content.
- The shared executor now has the TS streaming-fallback `discard()` path:
  queued/running tools are abandoned and returned as ordered synthetic
  `<tool_use_error>Error: Streaming fallback - tool execution discarded</tool_use_error>`
  results.
- Rust CLI and TUI model-result mapping now apply the TS empty-result guard:
  empty strings, empty arrays, and whitespace-only text block arrays become
  `(<ToolName> completed with no output)` instead of an empty tool result.
- Rust now has a shared model-facing tool-result formatter in
  `claude-core::tool_result_format`; the interactive TUI uses the same
  TS-style content mapping as print mode instead of sending raw JSON/stringified
  tool data back to the model.
- Rust now persists oversized textual tool results to the session
  `tool-results` directory and returns the TS `<persisted-output>` preview
  message instead of truncating inline content.
- Rust now enforces the TS aggregate per-message tool-result budget with
  in-memory seen/replacement state, including byte-identical replacement
  reapplication and infinite-cap tool opt-outs.
- Rust now writes TS-shaped `content-replacement` transcript entries for new
  aggregate-budget replacements and reconstructs replacement state on resume
  instead of treating those entries as conversation messages.
- Rust effort handling now follows TS levels (`low`, `medium`, `high`,
  `max`), no longer accepts/sends stale `xhigh`, and downgrades `max` to
  `high` unless the model supports max effort.
- Rust CLI effort resolution now follows TS precedence:
  `CLAUDE_CODE_EFFORT_LEVEL` overrides CLI/settings, and `auto`/`unset` clears
  explicit effort for the session.
- The `/effort` command and settings comments now expose the same supported
  effort set, so `xhigh` is rejected before it can become an API 400.
- The embedded TS tool-contract snapshot now matches the installed TS Bash
  co-author model label for the current Opus 4.6 path.
- MCP shadow contracts no longer override non-empty live MCP schemas, so
  connected Claude.ai/plugin servers keep the same live tool contracts TS sends.
- Rust now has a TS-style system-context lane in the query engine:
  `gitStatus: ...` is appended after the static system prompt via
  `appendSystemContext` formatting, rather than being mixed into user context.
- The old Rust static-prompt git-status helper was removed, so git status is
  no longer duplicated and no longer bypasses TS gates/truncation.
- CLI startup now gates git status the same way TS does:
  `CLAUDE_CODE_REMOTE` disables it, `CLAUDE_CODE_DISABLE_GIT_INSTRUCTIONS`
  truthy disables, explicitly falsy enables, and `includeGitInstructions`
  settings default to enabled.
- Rust git status now uses the TS recent-commit command shape
  `git --no-optional-locks log --oneline -n 5`.
- Unknown tool calls now use the TS model-facing error shape
  `<tool_use_error>Error: No such tool available: ...</tool_use_error>`
  instead of Rust-only `Unknown tool: ...` text.

Remaining result-budget work:
- Extend the same transcript-backed replacement state to sidechain/forked
  agent transcripts once Rust sidechain logging is wired like TS.

### Query user context still needs lifecycle parity

Files:
- `crates/claude-core/src/query/engine.rs`
- `crates/claude-core/src/context/system_prompt.rs`
- TS reference: `src/context.ts`, `src/utils/api.ts`,
  `src/utils/claudemd.ts`, `src/memdir/*`

Improved:
- CLI startup now appends request-time user context blocks for MCP server
  instructions, available skills, CLAUDE.md/rules aggregation, auto-memory,
  user email, and current date.
- CLAUDE.md loading honors `CLAUDE_CODE_DISABLE_CLAUDE_MDS`, bare mode, and
  additional-directory loading through the same gates as the TS flow.
- Dynamic context is prepended to the first user message or smooshed into the
  adjacent tool-result content, matching the TS message shape used in live
  proxy captures.

Needs work:
- Cache context at the same lifecycle points as TS.
- Verify remote/CCR resume behavior against live TS captures now that
  system-context injection is request-time instead of a static prompt mutation.

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

Improved:
- StopFailure hooks now fire for request errors, streamed API error events,
  and stream idle timeout failures instead of only the initial API send path.

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

Improved:
- `crates/claude-core/src/proxy/*` is now compiled/exported rather than dead
  code.
- Rust proxy env handling now follows TS `utils/proxy.ts` active-proxy
  precedence (`https_proxy > HTTPS_PROXY > http_proxy > HTTP_PROXY`) and uses a
  single active proxy URL for HTTP clients.
- Rust now reads/exports `NODE_EXTRA_CA_CERTS` alongside `SSL_CERT_FILE`,
  `REQUESTS_CA_BUNDLE`, and `CURL_CA_BUNDLE`.

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
- Proxy `NO_PROXY` URL matching, WebSocket proxy helpers, global HTTP-agent
  configuration, keepalive disable-on-reset behavior, mTLS helpers, AWS client
  proxy config, Anthropic unix-socket tunneling, and sandbox/upstream relay
  proxy integration.

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
- Wire MCP auth, elicitation, reconnect, and UI flows.

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
- File-history tracking for shell-modified files.
- Better command semantics for pipelines, shell builtins, and platform quirks.

Improved:
- Rust Bash permission checks now use the same read-only classifier as
  concurrency safety instead of a separate stale allowlist. This fixes the
  observed `cargo build 2>&1 | tail -5` mismatch: default mode now prompts
  like TS instead of treating the build as read-only.
- Foreground Bash commands now persist cwd across calls by capturing `pwd -P`
  after command completion, matching TS `runShellCommand` cwd tracking.
- Bash tool execution now carries the full `ToolPermissionContext`, so cwd reset
  uses the same allowed-working-path decision as TS instead of a local cwd-only
  check.
- Foreground Bash cwd now resets to the original project cwd when a command moves
  outside the allowed working directories, and appends `Shell cwd was reset to
  ...` to stderr like TS. `CLAUDE_BASH_MAINTAIN_PROJECT_WORKING_DIR` forces the
  reset without the warning suffix, matching TS.

### FileEditTool

Files:
- `crates/claude-tools/src/edit.rs`
- `crates/claude-tools/src/edit_quote_style.rs`
- TS reference: `src/tools/FileEditTool/*`

Needs work:
- Original-file capture in file history.
- LSP diagnostic clearing after edit.
- Settings-file validation.
- Multi-edit behavior and error messages parity.
- More tests for stale read state, partial reads, repeated replacements,
  newline handling, and quote-style preservation.

Improved:
- Conditional and nested dynamic skill activation now runs after Read/Edit/Write
  paths and registers newly discovered skills for the Skill tool.
- Rust `Edit` results now include the TS output keys `structuredPatch` and
  `userModified`, using the same shared display-diff helper as `Write`.
- CRLF preservation and LF-normalized `originalFile` behavior are covered by
  focused tests.
- Rust `Edit` now expands relative paths against the request cwd before
  permission checks, staleness checks, and filesystem writes.

### FileWriteTool

Files:
- `crates/claude-tools/src/write.rs`
- TS reference: `src/tools/FileWriteTool/*`

Improved:
- Rust `Write` results now include TS-shaped `structuredPatch`: updates return
  hunks with `oldStart`, `oldLines`, `newStart`, `newLines`, and prefixed diff
  lines; creates return `structuredPatch: []`.
- Existing empty files now follow TS's current truthiness behavior in the
  result shape: the write is reported as `create` with `originalFile: null`.
- Write permission safety prompts now mirror TS's narrow `.claude/skills/<name>`
  suggestion: Rust offers a session-scoped `Edit(/.claude/skills/<name>/**)`
  allow rule instead of a broad generic write-mode suggestion.
- Rust `Write` now expands relative paths against the request cwd before
  permission checks, staleness checks, and filesystem writes.
- Rust `Read`, `Edit`, and `Write` read-state timestamps now use the file's
  actual mtime like TS, instead of wall-clock "now", so stale-write checks and
  same-range Read dedup share the same timestamp basis.

Needs work:
- File-history integration.
- Conditional `gitDiff` output behind TS's remote/feature gate.
- Staleness atomicity and encoding/LSP/VSCode side effects in the exact TS
  call order.

### WebFetchTool

Files:
- `crates/claude-tools/src/web_fetch.rs`
- `crates/claude-tools/src/web_fetch_preapproved.rs`
- TS reference: `src/tools/WebFetchTool/*`

Improved since the stale report:
- Rust now accepts a `prompt` parameter and can call a secondary model.
- Rust now validates WebFetch URLs with the TS security shape (length cap,
  parseability, no embedded credentials, public-looking host), upgrades
  `http:` to `https:` before fetch, disables automatic cross-host redirects
  and returns the TS redirect instruction message, and emits the TS output
  keys (`bytes`, `code`, `codeText`, `result`, `durationMs`, `url`) without
  Rust-only prompt/debug fields.
- Rust now runs the TS domain-info preflight before fetching, with the same
  `skipWebFetchPreflight` settings escape hatch and domain-blocked/check-failed
  user-facing error text.
- Rust now sends the TS WebFetch request headers: `Accept: text/markdown,
  text/html, */*` and the `Claude-User (...)` WebFetch user agent.
- Rust now caches processed WebFetch content by original URL for 15 minutes
  before applying the current prompt, matching TS's URL cache placement before
  preflight/network fetch.
- WebFetch permission checks now use TS-style `domain:<hostname>` rule
  content, preapproved-host auto-allow, allow/ask/deny rule matching, and
  local-settings allow suggestions instead of inheriting blanket read-only
  auto-allow.
- HTML responses now go through an HTML-to-Markdown converter before prompt
  application, matching TS's Turndown-based flow much more closely than the
  previous manual tag stripper.
- Binary WebFetch responses now follow TS: raw bytes are saved to the
  session `tool-results` directory with a MIME-derived extension, cache entries
  preserve `persistedPath` / `persistedSize`, and the final result appends the
  TS saved-binary suffix.
- Rust no longer applies its old extra 50k-character WebFetch truncation before
  caching/secondary-model handling; truncation is left to the secondary prompt
  path like TS.
- WebFetch now treats non-2xx/non-handled-redirect HTTP responses as request
  failures like Axios does in TS, including the TS egress-proxy 403
  `EGRESS_BLOCKED` JSON error payload.
- Preapproved-host permission and prompt-path behavior has been checked against
  TS and uses the same host/path-prefix mechanism.
- WebFetch URL caching now follows TS's 15-minute, 50MB processed-content LRU
  behavior instead of an unbounded TTL-only map.

Still needs work:
- Exact Turndown formatting parity for edge-case HTML.
- Content-type/encoding details and size-limit behavior parity.
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
- Gate only if TS reintroduces `isWorktreeModeEnabled()` gating; the inspected
  TS reference currently enables worktree mode unconditionally.
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

Improved:
- Rust `ToolSearchTool` now returns the TS tool data shape
  `{matches, query, total_deferred_tools, pending_mcp_servers?}` instead of
  the older Rust-only `{matches, tools}` shape.
- Keyword search now searches deferred MCP tools rather than all loaded tools,
  while exact-name and `select:` queries can still return already-loaded tools
  as the TS no-op selection path does.
- Direct `mcp__server` prefix matching, case-insensitive `select:`, deferred
  count reporting, and model-facing `tool_reference` mapping are covered by
  focused tests.
- Rust now marks TS `shouldDefer` built-ins as deferred metadata too, so
  `ToolSearch` can discover tools such as `TodoWrite`, `WebFetch`,
  `WebSearch`, plan/task/cron/config/LSP tools, and worktree/team tools
  without request-time name guessing.
- Required-term searches now match TS scoring: a query containing only
  `+term` filters and scores on that required term instead of dropping all
  matches after filtering.
- Both MCP resource helper tools now use the TS deferred names in ToolSearch:
  `ListMcpResourcesTool` and `ReadMcpResourceTool`.

Needs work:
- Add integration coverage for `ToolSearch select:*` with a live/fake MCP
  server across multiple turns.

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
- Move `PreToolUse`, `PostToolUse`, and `PostToolUseFailure` dispatch into
  the shared tool executor instead of duplicating it in CLI/TUI loops.
- Fire compact, config, cwd, file, worktree, teammate, permission, and
  elicitation hooks at the same sites as TS.
- Implement aggregation/blocking behavior exactly.
- Ensure hook outputs are inserted into transcript/context like TS.

Improved:
- Rust now fires `StopFailure` on non-prompt-too-long API failures.
- Rust now fires main-thread `Stop` hooks after normal assistant turns,
  injects blocking feedback as user messages, continues the query when a
  Stop hook blocks, and respects `preventContinuation`.
- Rust task tools now fire `TaskCreated` and `TaskCompleted` hooks around task
  create/update.
- Rust print mode now fires `SessionStart` and `UserPromptSubmit` hooks before
  the first model request; `UserPromptSubmit` blocking errors prevent the
  prompt from reaching the model, and additional context is injected into the
  request-time user context lane. `UserPromptSubmit` hook execution is not
  emitted as stream-json hook events, matching the observed TS print stream.
- Rust print mode now applies `SessionStart.initialUserMessage` as an initial
  user turn before the submitted prompt, matching TS headless orchestrator
  startup.
- Rust TUI regular prompt submission now fires `UserPromptSubmit` before
  sending the message to the model, with blocking/prevent-continuation handling
  and additional-context injection.
- Rust now fires `CwdChanged` hooks and updates the process hook runner cwd
  when CLI or TUI worktree tools move the session between the original cwd and
  a worktree, following the TS cwd-watcher hook path.
- Rust print mode now accepts the TS `--json-schema` flag, injects a
  schema-specific `StructuredOutput` tool, validates its input with a JSON
  Schema validator, and includes `structured_output` on successful JSON and
  stream-json result events. It also enforces the TS stop-hook retry loop that
  requires the model to call `StructuredOutput`, honoring
  `MAX_STRUCTURED_OUTPUT_RETRIES` with the TS error subtype.
- Rust print mode now accepts `--system-prompt`, `--system-prompt-file`, and
  `--append-system-prompt-file` with the same mutual-exclusion/file-read
  behavior as TS; a custom system prompt replaces the default prompt and append
  prompt content is appended afterward.
- Rust print mode now accepts the TS SDK input flags `--input-format`,
  `--include-hook-events`, `--include-partial-messages`, and
  `--replay-user-messages`, validates the stream-json-only combinations, and
  seeds the initial prompt, system prompt, append prompt, and JSON Schema from
  SDK stream-json stdin `user` / `initialize` messages.
- Rust startup now accepts TS runtime configuration flags `--settings`,
  `--mcp-config`, and `--strict-mcp-config`: settings are layered over loaded
  settings, dynamic MCP configs are parsed from JSON strings or files with
  environment-variable expansion, and CLI MCP configs override file/plugin
  configs while preserving insertion order.
- Rust settings `mcpServers` now parse the same transport union as TS instead
  of only the legacy stdio shape, so user/project/runtime settings can define
  `stdio`, `http`, `sse`, `sse-ide`, `ws`, and `ws-ide` entries through the
  same general MCP config path.
- Rust startup now accepts TS `--setting-sources` and applies the same source
  names (`user`, `project`, `local`) to typed settings, raw permission settings,
  MCP settings, and hook input. Policy settings and `--settings` remain
  always-on layers like TS.
- Rust print mode now accepts TS `--task-budget` and `--workload`: task
  budgets are sent as `output_config.task_budget` with the task-budgets beta,
  and workload tags are threaded into the OAuth billing attribution block as
  `cc_workload`, matching the TS request-construction mechanism.
- Rust print mode now accepts TS `--betas` and applies the same SDK beta
  allowlist behavior: only `context-1m-2025-08-07` is forwarded, disallowed
  values warn and are ignored, and OAuth/subscriber sessions ignore custom
  betas like TS.
- Rust print mode now accepts TS `--max-budget-usd`, validates positive
  numeric input, checks accumulated request cost after turns, and emits the
  TS `error_max_budget_usd` result shape/text when the cap is reached.
- Rust print mode now accepts TS `--fallback-model`, rejects a fallback equal
  to the active main model, threads it into API config, and switches the
  outgoing model after repeated 529 overload responses using the same retry
  budget constant as TS.
- Rust startup now accepts TS `--bare` and sets `CLAUDE_CODE_SIMPLE=1` before
  runtime loading, so existing simple-mode gates apply to memory, skill
  discovery, background work, hooks, and plugin MCP auto-loading while explicit
  CLI inputs such as `--mcp-config`, `--settings`, and `--add-dir` still work.

Still needs work:
- Persist and render full Stop hook progress/summary attachment messages like
  TS.
- Wire `SubagentStop`, `TeammateIdle`, and `TaskCompleted` end-of-turn hooks.

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

Improved:
- Bash permission evaluation now routes unknown/mutating commands through the
  normal ask flow in default mode, with regression coverage for `cargo build`.

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
- Full remote-control/session manager behavior. Rust now exposes the
  `remote-control` / `rc` CLI surface and `/remote-control` TUI command, but
  it is intentionally a guarded stub until the real TS bridge runtime is
  ported. The CLI entrypoint shape, help text, accepted flags, hidden root-help
  behavior, and bridge flag validation now match the installed TS fast path.
  Still missing: entitlement/policy checks, environment registration, session
  creation/reconnect, session-ingress WebSocket forwarding, inbound web/mobile
  prompt queueing, and connected/disconnect lifecycle.
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
- Bridge dialogs and live Remote Control state. `/remote-control` exists in
  Rust and records the local request in shared command state, but it cannot yet
  show the TS `/remote-control is active · Code in CLI or at ...` callout
  because no `session_*` URL is created without the bridge runtime.
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
4. Verify rich user/system context lifecycle against resumed sessions and
   remote-control captures.
5. Complete the remaining stream-json rate-limit timing parity by plumbing
   API status changes like TS instead of relying on deterministic fallback
   placement around very fast `PreToolUse` hooks.
6. Add transcript-level tests for query/tool/cancel/compact/resume behavior.
7. Decide scope for bridge/direct-connect/upstream proxy before porting more
   isolated helpers.
8. Update or replace `feature-gap-analysis.md` once this checklist is validated.
