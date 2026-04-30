# Rust Rewrite vs Original TS: Missing / Needs Work

Date: 2026-04-28

Latest live proxy comparison: 2026-04-29

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

Captured through `scripts/run_parity_capture.py` on 2026-04-29:

- Capture directory:
  `/tmp/claude-rs-parity-after-powershell`
- Fresh fixed-session lifecycle capture:
  `/tmp/claude-rs-context-lifecycle-start-3`
- Resume lifecycle capture:
  `/tmp/claude-rs-context-lifecycle-resume-3`
- Remote-mode context capture (`CLAUDE_CODE_REMOTE=1`):
  `/tmp/claude-rs-context-lifecycle-remote-mode-2`

Now matching:

- `model`, `max_tokens`, `stream`, `thinking`, and `context_management`.
- `output_config` effort now uses TS-supported values only; stale `xhigh`
  settings/env values are coerced before reaching the API.
- Message shape.
- Resume lifecycle shape: Rust now persists normal user/assistant/tool
  messages, reuses the resumed session id for non-fork `--resume`, reconstructs
  the TS in-memory continuation sentinel pair (`Continue from where you left
  off.` / `No response requested.`), and avoids duplicating startup context
  when the resumed history already carries it. Latest resume capture is
  `message_count` 5 vs 5 and `message_shape` equal.
- Remote-mode context gates: with `CLAUDE_CODE_REMOTE=1`, Rust now hides
  `RemoteTrigger`, hides the bundled `schedule` skill, skips git system
  context, and matches TS tool count/order (86 vs 86).
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
- Rust stream-json now follows TS rate-limit status flow: `ApiClient` extracts
  quota status from successful response headers and 429 error headers, updates a
  process-wide rate-limit status listener set, and stream-json subscribes to
  that listener to emit `rate_limit_event`. The old deterministic fallback
  emissions after `Done`, `ToolUse`, and `MaxTurns` were removed.
- Query transcript coverage now exercises the real API/SSE path with a local
  test server: normal user+assistant turns, assistant tool-use plus follow-up
  tool_result, cancellation after tool_use with synthetic tool_result
  persistence, content-replacement transcript records plus resume
  reconstruction, and compact-boundary resume filtering. This also fixed two
  parity bugs: cancelled tool-use synthetic results are now written to the
  transcript, and compact-boundary entries survive the resume splitter before
  TS-style post-boundary filtering runs.
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
- Latest fresh/resume/remote lifecycle captures match message shape and context
  lifecycle. Full scrubbed-body equality still reports `no` because private
  plugin skill ordering remains nondeterministic and current live Agent-tool
  metadata differs for some plugin-provided agents; this is tracked outside the
  context lifecycle item.

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

### Tool registry exposure/gating

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
- Rust now uses the full `ToolPermissionContext` for prompt-visible registry
  filtering, not just settings-file deny rules. This matches TS
  `filterToolsByDenyRules` across CLI, disk, policy, and MCP server-prefix
  deny sources.
- ToolSearch snapshot refresh now removes any stale snapshot before rebuilding
  and is filtered again afterward, so a denied or CLI-excluded ToolSearch is
  not reintroduced after MCP registration.
- Prompt/tool-definition ordering now follows TS `assembleToolPool`: sorted
  built-ins as one prefix, then sorted MCP-partition tools. MCP resource helper
  tools (`ListMcpResourcesTool`, `ReadMcpResourceTool`) are treated as MCP
  partition tools, matching TS `specialTools` / `appState.mcp.tools` behavior.
- MCP resource tools remain absent from the base registry and are added only
  after a connected MCP server advertises resource capabilities, matching TS
  MCP connection-state visibility.
- Added registry tests for full-context deny filtering, MCP server-prefix deny
  filtering, ToolSearch not being reintroduced after deny, and MCP resource
  tool ordering.

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
- Core built-in tools now expose TS-style `toAutoClassifierInput`
  projections through the Rust tool trait for Bash, PowerShell, Read, Edit,
  Write, Glob, Grep, WebFetch, and Task tools instead of relying on raw JSON
  classifier input.

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
- Rust now materializes first-turn context into the durable message lifecycle
  before the first API request, so resumed sessions keep the same model-visible
  context prefix as TS.
- Print-mode `--resume` now reuses the resumed session id, not a fresh CLI
  session id.
- Print-mode `--resume` now adds TS-style in-memory continuation markers
  without writing them back to the transcript.
- Remote-mode (`CLAUDE_CODE_REMOTE=1`) now hides remote-trigger scheduling
  surfaces that TS does not expose in remote sessions.

Remaining adjacent work:
- Remote-control's actual session-ingress bridge runtime is still tracked in
  the bridge/direct-connect scope below; this item verified the remote-mode
  context gates available through live API captures.

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
- Extend transcript-level tests to remaining query edges not covered by the
  current real-SSE harness: max-token recovery, stop-hook blocking/retry, and
  prompt-too-long reactive compaction.

Improved:
- StopFailure hooks now fire for request errors, streamed API error events,
  and stream idle timeout failures instead of only the initial API send path.
- Transcript-level tests now cover normal query turns, tool-use turns,
  cancellation after tool_use, large tool-result content-replacement records,
  compact-boundary resume filtering, and ephemeral resume continuation markers.

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
- Rust proxy matching now includes the TS `shouldBypassProxy` / WebSocket proxy
  decision rules: `no_proxy > NO_PROXY`, wildcard `*`, comma/space-separated
  entries, exact host/IP matches, leading-dot domain suffixes, and
  port-specific `host:port` entries.
- Rust now has TS-shaped direct-connect session creation and StructuredIO
  framing helpers for the `src/server/createDirectConnectSession.ts` /
  `directConnectManager.ts` contract: `POST /sessions`, bearer auth, optional
  `dangerously_skip_permissions`, SDK user-message encoding, permission
  response encoding, interrupt encoding, ignored stdout message filtering, and
  unsupported control-request classification.
- Rust direct-connect now includes a WebSocket runtime loop that applies bearer
  auth, reads newline-delimited StructuredIO frames, emits parsed inbound
  messages/permission requests, and writes TS-shaped user, permission,
  interrupt, and error responses.
- Rust now has a general bridge environments API client matching
  `src/bridge/bridgeApi.ts` for registration, polling, ack, stop, deregister,
  archive, reconnect, heartbeat, and permission response events, including the
  TS bridge beta/version headers, trusted-device header slot, safe path-ID
  validation, 409 archive idempotency, and TS-style fatal error mapping.
- Rust now has the org-scoped bridge Sessions API surface from
  `src/bridge/createSession.ts`: create/fetch/archive/title-update session
  calls, `ccr-byoc-2025-07-29` headers, organization UUID header, remote-control
  session body shape, GitHub source/outcome context building, `cse_*` to
  `session_*` title-update compatibility, and best-effort archive/title update
  behavior.
- Rust now includes CCR session-ID tag helpers (`cse_*` ⇄ `session_*`) and the
  CCR v2 `registerWorker` call next to the existing work-secret URL helpers.
- `claude remote-control` now consumes those bridge clients directly: it builds
  the TS-shaped runtime config from the current directory, branch, origin
  remote, OS hostname, spawn/capacity flags, sandbox/debug/timeout flags, then
  registers the environment, optionally pre-creates the initial session, prints
  the compat session URL, polls for bridge work, decodes work secrets,
  acknowledges healthchecks, launches TS-shaped child sessions with `--sdk-url`
  / stream-json flags and Session-Ingress environment variables, updates
  running child tokens on redelivery, heartbeats active work, stops work on
  failure/shutdown, archives known sessions, and deregisters the environment.
  Rust print mode now also recognizes child `--sdk-url` sessions, connects to
  the v1 Session-Ingress WebSocket, applies the same token-refresh environment
  messages TS `StructuredIO` accepts, derives the TS `HybridTransport` POST
  endpoint from the WebSocket URL, and mirrors stream-json stdout events to
  Session-Ingress HTTP POST batches. It also selects the CCR v2 shape when
  `CLAUDE_CODE_USE_CCR_V2` is set: the child reads `client_event` payloads from
  `{sessionUrl}/worker/events/stream` and posts wrapped client events to
  `{sessionUrl}/worker/events` with the worker epoch. Remote child print mode
  now keeps the input stream alive after the first prompt, queues later remote
  `user` messages for subsequent turns, forwards `can_use_tool` permission
  requests with the SDK control-request shape including suggestions/blocked
  path, and applies matching remote `control_response` allow/deny decisions.
  Remote allow responses now apply updated input, persist updated permissions,
  preserve EnterWorktree/ExitWorktree cwd side effects, and keep user messages
  that arrive during a permission wait queued for later turns. The CCR v2 child
  transport also reports worker state transitions (`running`, `requires_action`,
  `idle`) from the same event classes TS uses and sends periodic
  `/worker/heartbeat` requests with the session id and worker epoch. CCR v2 SSE
  `client_event` frames now preserve the server `event_id`, enqueue `received`
  delivery updates to `/worker/events/delivery` using the same batch body shape
  as TS, and carry user-message event ids through the prompt loop so turn start
  and completion report `processing`/`processed`. Worker init now clears stale
  CCR `external_metadata.pending_action`/`task_summary`, and permission-blocked
  state patches mirror `pending_action` metadata from the same control-request
  details TS uses. CCR v2 transcript writes now also use the shared
  `SessionStorage` path to enqueue TS-shaped worker internal events to
  `/worker/internal-events`, preserving the original transcript payload type,
  adding a `uuid` when missing, and carrying generic compaction/agent metadata.
  In CCR v2 print-mode resume with `--sdk-url`, Rust now also reads paginated
  foreground internal events from `/worker/internal-events` before loading the
  transcript and rewrites the local transcript from the returned payloads like
  TS `hydrateFromCCRv2InternalEvents`. CCR v2 outbound `stream_event` writes now
  also use the TS 100ms delay/coalescing mechanism for text deltas: each flush
  emits one full-so-far snapshot per touched text block, non-stream messages
  flush pending stream events first, and the final assistant message clears the
  active accumulator. Rust now also routes runtime `set_model` and
  `set_permission_mode` control requests through the print loop, updates the
  active model/permission context for later turns, emits TS-shaped
  `control_response` messages, and reports CCR `external_metadata.model`,
  `permission_mode`, and `is_ultraplan_mode` through the worker metadata path.

Missing or partial:
- Full bridge messaging.
- `initReplBridge` / `replBridge` behavior.
- `remoteBridgeCore` parity.
- Full CCR v2 client lifecycle depth. Rust now has the basic child
  `SSETransport`/event-write shape for `CLAUDE_CODE_USE_CCR_V2`, but still
  needs the remaining TS `CCRClient` depth around subagent internal-event restore
  and the parent/hosted bridge lifecycle.
- Bridge permission callbacks are partially wired for remote child
  `can_use_tool` requests/responses. Remaining depth is parent-side forwarding
  through the hosted bridge permission API and cancellation/delivery lifecycle.
- Trusted-device flow.
- Full work secret lifecycle and token-refresh scheduling.
- Capacity wake.
- Flush gate.
- Poll config.
- Inbound attachments/messages.
- Bridge UI.
- Direct WebSocket session manager integration into CLI/TUI entrypoints.
- Upstream relay/proxy.
- Proxy global HTTP-agent configuration, keepalive disable-on-reset behavior,
  mTLS helpers, AWS client proxy config, Anthropic unix-socket tunneling, and
  sandbox/upstream relay proxy integration.

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

Improved:
- MCP auto-mode classifier input encoding now follows the TS
  `mcpToolInputToAutoClassifierInput` mechanism, including insertion-order
  keys and JavaScript `String(value)` coercion for arrays and nested objects.
- The Rust permission/classifier bridge now has a tool-level
  `to_auto_classifier_input` hook and MCP tools route through the TS-style MCP
  projection instead of sending raw JSON to the classifier.

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
- Foreground Bash results now use the shared TS command-semantics classifier:
  non-error informational exit codes such as `grep` no-match stay successful
  with `returnCodeInterpretation`, real non-zero failures become error tool
  results, and silent commands set `noOutputExpected`.
- Bash assistant auto-background model-facing text now uses the TS 15-second
  blocking budget wording.
- Bash model-facing stdout now removes leading blank/whitespace-only lines and
  trims trailing newlines like TS `mapToolResultToToolResultBlockParam`.

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
- Rust `Edit` now includes TS-shaped `gitDiff` output only when both
  `CLAUDE_CODE_REMOTE` and the cached GrowthBook flag
  `tengu_quartz_lantern` are enabled, using the TS single-file diff rules:
  tracked files diff against `CLAUDE_CODE_BASE_REF` or the default-branch
  merge-base, while untracked files get a synthetic all-additions patch.

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
- Rust `Write` now includes TS-shaped `gitDiff` output behind the same
  `CLAUDE_CODE_REMOTE` plus cached `tengu_quartz_lantern` GrowthBook gate as
  TS, sharing the tracked/untracked single-file diff helper with `Edit`.

Needs work:
- File-history integration.
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
- WebFetch secondary-model failures now fail the tool call instead of silently
  returning raw fetched content, and empty secondary-model text maps to the TS
  `No response from model` fallback.
- WebFetch now converts fetched content to Markdown only when the HTTP
  `content-type` includes `text/html`, matching TS; Rust no longer guesses HTML
  from a leading `<` in non-HTML responses.

Still needs work:
- Exact Turndown formatting parity for edge-case HTML.
- Content-type/encoding details and size-limit behavior parity.
- Copyright/quote-limit behavior exactly matching TS.
- Wire the Haiku secondary model in every runtime path that can execute
  WebFetch, not only CLI startup.

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

Completed in Rust:
- Agent input schema now matches the TS externally visible contract for required `description` + `prompt`, the legacy `Task` alias, and `CLAUDE_CODE_DISABLE_BACKGROUND_TASKS` hiding `run_in_background`.
- Agent auto-classifier input now matches TS `toAutoClassifierInput`: `(subagent_type, mode=...): prompt`, or `: prompt`.
- Agent spawn now honors TS-style `Agent(agentType)` deny rules at call time.
- Child tool resolution now follows TS `resolveAgentTools`: start from the parent available tools, remove globally disallowed subagent tools, apply async allow-list when running in background, parse permission-rule tool specs, and apply each agent's allow/deny frontmatter before passing `--tools`.
- Background Agent result shape now matches TS async launch fields (`isAsync`, `status`, `agentId`, `description`, `prompt`, `outputFile`, `canReadOutputFile`) and uses the task id as the agent id.
- Background Agent output is persisted to a task output file and task status is marked completed/failed when the child exits.
- Agent prompt generation now uses the embedded TS Agent contract as its source and swaps in the runtime-filtered agent list, avoiding a separately maintained Rust prompt.
- Tool definitions can now be built with a runtime `ToolDefinitionContext`, matching the TS Agent prompt inputs (`agents`, `tools`, `toolPermissionContext`, `allowedAgentTypes`).
- Prompt-time filtering now matches TS: agents are filtered by required MCP servers, `Agent(agentType)` deny rules, and allowed agent types before the model sees the Agent tool description.
- Markdown/plugin/CLI agents now carry `requiredMcpServers`.
- `Agent(x,y)` tool-spec metadata is preserved through child tool resolution and propagated to spawned child sessions.
- Spawned agents now get stable `--session-id` values matching their foreground/background agent id, so transcript/session storage is tied to the agent id instead of an unrelated random CLI session.
- Fork-mode and `isolation: "remote"` are not exposed by the embedded external TS 2.1.119 Agent tool contract used as the parity source in this repo; Rust therefore keeps them out of the public Agent prompt/schema for this build.

### Task tools / Todo tools

Files:
- `crates/claude-tools/src/task_tools.rs`
- `crates/claude-tools/src/todo_write.rs`
- TS reference: `src/tools/TaskCreateTool/*`,
  `src/tools/TodoWriteTool/*`, `src/utils/tasks.ts`

Needs work:
- TS file-lock task mutation serialization.
- Full owner/team mailbox behavior and automatic teammate owner assignment.
- Active spinner UI behavior using task `activeForm`.
- Cross-session task behavior.

Improved:
- Rust now matches TS Task v2/TodoWrite visibility for interactive vs
  noninteractive startup, including `CLAUDE_CODE_ENABLE_TASKS`.
- TaskCreate, TaskGet, TaskList, and TaskUpdate now return the same
  model-facing result envelopes as TS (`{task: ...}`, `{tasks: ...}`,
  `{success, taskId, updatedFields, statusChange...}`) instead of leaking the
  internal task store entry shape.
- TaskCreate and TaskUpdate are now marked concurrency-safe like TS.
- Rust task tools now fire `TaskCreated` and `TaskCompleted` hooks around task
  create/update.
- Tasks are now persisted under the Claude config task directory keyed by
  `CLAUDE_CODE_TASK_LIST_ID`, `CLAUDE_CODE_TEAM_NAME`, or the session id,
  matching the TS task-list resolution shape for standalone sessions and
  process-based teammates.
- Task ID allocation now honors a TS-style `.highwatermark` file so deleted
  task IDs are not reused.
- TaskCreate and TaskUpdate now persist TS task fields for `activeForm`,
  `owner`, `blocks`, `blockedBy`, and `metadata`, including metadata null-key
  deletion.
- TaskUpdate now applies `addBlocks` / `addBlockedBy` through the same
  bidirectional dependency model as TS, and TaskGet / TaskList expose the
  resulting dependency fields.
- TaskList now filters internal metadata tasks and hides completed blockers
  from `blockedBy`, matching TS list output behavior.
- Task deletion now removes dependency references from other tasks and updates
  the high-water mark, matching TS delete cleanup semantics.
- Task tool auto-classifier projections now match TS (`subject`, task id, and
  update id/status/subject summaries).
- TaskStop and TaskOutput now use TS-style `task_id` input contracts and
  model-facing output envelopes, while still accepting the older Rust `taskId`
  field for compatibility.

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
- Confirm Windows-specific behavior.
- Permission and read-only semantics parity with Bash.
- Background/progress, cwd reset, large-output persistence, git tracking, and
  image-output handling from TS `PowerShellTool.call`.

Improved:
- Rust already gates registration through the shared
  `is_powershell_tool_enabled()` helper, matching TS `isPowerShellToolEnabled()`.
- PowerShell external-command exit semantics now follow TS
  `PowerShellTool/commandSemantics.ts` for `grep`/`rg`/`findstr` no-match and
  `robocopy` success bitfields.
- Model-facing PowerShell results now use the TS shell stdout/stderr mapping
  instead of exposing raw JSON.

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

Completed audit:
- Rust now has registry-level alias support and TS-compatible aliases:
  `/settings`, `/continue`, `/checkpoint`, `/bashes`, `/allowed-tools`,
  `/plugins`, `/marketplace`, `/app`, `/ios`, `/android`, `/bug`, `/quit`,
  `/remote`, and `/rc`.
- Rust now uses TS canonical names for `/terminal-setup`, `/think-back`, and
  `/web-setup`, while keeping previous Rust names as compatibility aliases.
- Added placeholder slash commands for TS surface entries that were absent from
  Rust: `/advisor`, `/exit`, `/ide`, `/login`, `/logout`,
  `/rate-limit-options`, `/statusline`, `/stickers`, `/vim`, and
  `/thinkback-play`.
- Added tests that required commands exist and aliases resolve to the canonical
  TS command.

Classification:

| Command(s) | Status | Notes |
| --- | --- | --- |
| `/help` | partial | Static Rust help text; TS renders rich Ink help and dynamic command availability. |
| `/clear` (`/reset`, `/new`) | full | Clears Rust conversation state and alias behavior now matches TS. |
| `/cost`, `/stats`, `/usage` | partial | Rust reports local tracker data; TS has richer account/session usage surfaces. |
| `/model`, `/effort`, `/fast`, `/brief`, `/theme`, `/color` | partial | Rust mutates shared TUI state/settings; TS uses interactive pickers and richer validation. |
| `/config` (`/settings`) | partial | Rust shows settings text; TS opens config UI. |
| `/permissions` (`/allowed-tools`) | partial | Rust shows permission mode; TS has full permission management UI. |
| `/memory` | partial | Rust lists memory files; TS has full memory editor/discovery behavior. |
| `/tasks` (`/bashes`) | partial | Rust lists task state; TS has richer task/background-command dialogs. |
| `/resume` (`/continue`) | partial | Rust has command stub/session logic elsewhere; TS has picker/search/resume UX. |
| `/context` | partial | Rust reports token/context values; TS has visual context grid and noninteractive variant. |
| `/doctor` | partial | Rust performs basic local checks; TS doctor has broader install/auth/IDE/MCP diagnostics. |
| `/diff` | partial | Rust shows git diff; TS UI has richer rendering. |
| `/export` | partial | Rust writes markdown/text export; TS has export dialog/session-content parity still to verify. |
| `/mcp` | partial | Rust status/listing only; TS has full MCP management UI. |
| `/plugin` (`/plugins`, `/marketplace`) | partial | Rust local listing/help only; TS marketplace/install/enable/disable UI is not ported. |
| `/skills` | partial | Rust lists skills; TS has richer skills UI/search. |
| `/agents` | partial | Rust lists minimal agent/task state; TS has running-agent management dialogs. |
| `/rewind` (`/checkpoint`) | partial | Rust has rewind UI elsewhere, but command-level behavior is not fully TS. |
| `/files` | partial | Rust lists project/context files; TS ant-only file context UI differs. |
| `/hooks` | partial | Rust lists hook config; TS has hook command UI/flows. |
| `/session` (`/remote`) | partial | Rust shows session info; TS remote session URL/QR flow is richer. |
| `/copy` | partial | Rust uses shell clipboard tools; TS has selector/UI behavior. |
| `/rename`, `/add-dir`, `/tag` | partial | Rust updates local shared state; TS has richer validation/UI. |
| `/release-notes`, `/reload-plugins` | partial | Rust returns text/local reload summary; TS has richer plugin/release flows. |
| `/sandbox`, `/output-style` | partial | Rust has local settings stubs; TS has full sandbox/output-style UI. |
| `/remote-control` (`/rc`) | partial | Rust records local request; TS starts Claude.ai session-ingress bridge. |
| `/remote-env` | partial | Rust prints env values; TS configures remote environment. |
| `/exit` (`/quit`), `/vim`, `/statusline`, `/stickers` | partial | Rust placeholders; TS executes TUI/input/status/sticker flows. |
| `/ide` | partial | Rust placeholder; TS manages IDE integrations. |
| `/login`, `/logout` | partial | Rust points to CLI subcommands; TS opens auth dialogs. |
| `/feedback` (`/bug`) | link-only | Rust prints issue URL/transcript text; TS opens feedback/bug flow. |
| `/upgrade`, `/privacy-settings`, `/install-github-app`, `/install-slack-app`, `/chrome`, `/desktop` (`/app`), `/mobile` (`/ios`, `/android`), `/terminal-setup`, `/heapdump`, `/web-setup` | link-only | Rust gives URLs/manual instructions; TS opens product-specific UI/wizards. |
| `/init`, `/init-verifiers`, `/compact`, `/plan`, `/exit-plan`, `/commit`, `/review`, `/branch`, `/pr-comments`, `/commit-push-pr`, `/security-review`, `/insights`, `/btw`, `/advisor`, `/passes` | prompt-only | Rust expands to a prompt; TS prompt commands may also carry allowed-tools/progress metadata. |
| `/ultrareview`, `/think-back`, `/thinkback-play`, `/rate-limit-options`, `/extra-usage` | partial | Product/account/UI-backed TS behavior is only approximated or placeholder in Rust. |
| `/proactive`, `/ultraplan`, `/share`, `/env`, `/team-onboarding`, `/test`, `/refactor`, `/explain`, `/docs` | Rust-only/divergent | Not part of the inspected external TS command list or are internal/gated in TS; keep under review so they do not leak when TS would hide them. |
| `/voice`, `/assistant`, `/brief` TS command, `/peers`, `/workflows`, `/torch`, `/buddy`, `/remote-control-server`, `/version`, internal-only commands | missing or gated out | TS registers these only behind feature/user/build gates. Rust should only expose them if the corresponding subsystem is ported and gated the same way. |

Remaining slash-command work:
- Replace partial/link-only placeholders with real TS-equivalent TUI dialogs and
  service calls where product parity is required.
- Add golden tests against TS output/command metadata for stable local and
  prompt commands.
- Port command availability filtering (`availability`, `isEnabled`, provider
  gates, remote-safe/bridge-safe filtering) as a general command registry layer.

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
  `remote-control` / `rc` CLI surface and `/remote-control` TUI command. The
  CLI entrypoint shape, help text, accepted flags, hidden root-help behavior,
  bridge flag validation, TS-shaped spawn/capacity defaults, sandbox/debug/
  timeout option mapping, OS hostname/current-branch/remote collection,
  environment registration, initial session creation, remote session URL
  printing, bridge work polling, healthcheck ack, session work-secret decode,
  TS-shaped child process launch, token update forwarding, heartbeat, stopWork,
  archive, deregister, v1 child Session-Ingress WebSocket/POST bridge, basic
  CCR v2 child SSE/client-event POST bridge, multi-turn remote child prompt
  queueing, and child-side `can_use_tool` permission control responses
  including updated input/permissions and worktree cwd side effects, plus CCR
  worker state reports for running/requires-action/idle, explicit
  `--session-id` reconnect via `getBridgeSession` + environment reuse +
  `/bridge/reconnect`, and `--continue` bridge-pointer lookup/write/clear for
  single-session standalone remote control, are now wired to the general bridge
  API/session clients. CCR v2 child sessions now also send `/worker/heartbeat`
  and delivery updates for SSE `client_event` ids (`received`, plus
  user-message `processing`/`processed`), and mirror pending-action
  `external_metadata` for permission waits.
  Still missing:
  entitlement/policy checks, full CCR v2 lifecycle reporting, parent-side
  hosted permission callback forwarding, trusted-device flow, and
  connected/disconnect TUI state.
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

1. Decide scope for bridge/direct-connect/upstream proxy before porting more
   isolated helpers.
2. Reconcile current live Agent-tool metadata for plugin-provided agents.
3. Update or replace `feature-gap-analysis.md` once this checklist is validated.
