# Feature Gap Analysis — `claude-code-leaked/src` (TS) vs `claude-rs` (Rust)

> Fresh full re-analysis, 2026-04-18. Supersedes the 2026-04-01 version.

Comparing the TypeScript reference implementation at
`claude-code-leaked/src/` (1902 files, ~33 MB) against the Rust port at
`claude-rs/crates/{claude-cli, claude-core, claude-tools, claude-tui}/`
(~415 .rs files, ~2.8 MB).

**Headline:** Rust has strong core-engine + tools parity (all 42 TS tools
mapped), but is thin or missing on UI, memory/migrations, command handlers,
constants/prompts, bridge/proxy, and permission plumbing. Rust also adds
several subsystems the TS side does not have (Teams, Sandbox, LSP depth,
VCR, Workflows, File History).

---

## 1. Tools (`src/tools/` ↔ `crates/claude-tools/src/`)

### Coverage
All 42 TS tools have a Rust file. Rust additionally has tools that in TS live
outside `/tools/` as feature-gated injections: MonitorTool,
PushNotificationTool, SubscribePRTool, SendUserFileTool, ListPeersTool,
CtxInspectTool, TerminalCaptureTool, SnipTool, VerifyPlanTool, WorkflowTool.

### Missing tools
**None fully missing.**

### Depth gaps (major tools)

| Tool | TS | Rust | Notable missing in Rust |
|---|---|---|---|
| **BashTool** | ~632 KB (bash + `bashPermissions` 96 KB + `readOnlyValidation` 66 KB + `sedEditParser` + `sedValidation`) | `bash.rs` 52 KB + `bash_security.rs` | file-history tracking, chunked streaming output, full per-command permission matrix, sed parser/validator, background-task spawn (`spawnShellTask`, `registerForeground`), git-op tracking, cwd-reset-if-outside-project |
| **FileEditTool** | 96 KB | `edit.rs` 8 KB | git diff output (`gitDiffSchema`), read-state staleness check, original-file capture, CRLF preservation, LSP diagnostic clearing, settings-file validation, quote-style preservation, conditional-skill activation |
| **FileWriteTool** | ~20 KB | `write.rs` 10 KB | staleness check, write permission validation, file history, git diff output, append mode |
| **WebFetchTool** | ~52 KB | `web_fetch.rs` 7 KB | **prompt param ignored** (no `applyPromptToMarkdown`), no preapproved-host allowlist, no domain permission gating, returns plain text not markdown, no URL scheme validation |
| **SkillTool** | ~76 KB | `skill_tool.rs` 12 KB | dynamic skill discovery, conditional skill activation by path, dynamic skill-dir addition, MCP-based skill execution |
| **TaskCreateTool** | ~12 KB suite | `task_tools.rs` 20 KB | `TaskCreated` hooks not fired, external persistence (TS uses Notion-like list; Rust is in-memory), owner/team awareness, blocks/blockedBy, `activeForm` spinner verb, auto-classifier input |
| **AgentTool** | 572 KB | `agent_tool.rs` 24 KB + `agents/` | **fork mode not implemented** (source comment: "currently non-fork path"), thinner spawn integration, minimal progress reporting |

### Registry / gating (`tools.ts` ↔ `registry.rs`)
TS `tools.ts` gates on `USER_TYPE`, feature flags, env vars (PROACTIVE, KAIROS,
AGENT_TRIGGERS, MONITOR_TOOL, TODO_V2, WORKTREE_ENABLED, ENABLE_LSP_TOOL,
CLAUDE_CODE_VERIFY_PLAN, …), applies `filterToolsByDenyRules`, and hides
`REPL_ONLY_TOOLS` in REPL mode.
Rust `registry.rs` is a plain HashMap — **no feature-flag gating, no deny-rule
filter, no alias population, no REPL hiding**. Flag-gated TS tools are
unconditionally registered in Rust.

---

## 2. Commands (`src/commands/` ↔ `crates/claude-core/src/commands/`)

TS has **77 slash commands**. Rust fully implements ~5 (`help`, `status`,
`model`, `clear`, `cost`) with ~43 more listed in help text but without
handlers.

### Fully missing in Rust (29)
`agents`, `btw`, `chrome`, `copy`, `desktop`, `effort`, `export`, `extra-usage`,
`feedback`, `heapdump`, `install-github-app`, `install-slack-app`,
`keybindings`, `mobile`, `output-style`, `passes`, `pr_comments`,
`privacy-settings`, `rate-limit-options`, `release-notes`, `reload-plugins`,
`remote-env`, `remote-setup`, `stats`, `stickers`, `tag`, `terminalSetup`,
`thinkback`, `thinkback-play`, `upgrade`, `usage`.

### Listed but no handler
`add-dir`, `branch`, `bridge`, `color`, `compact`, `config`, `context`, `diff`,
`doctor`, `files`, `hooks`, `ide`, `login`, `logout`, `mcp`, `memory`,
`permissions`, `plan`, `rename`, `resume`, `review`, `rewind`, `sandbox-toggle`,
`session`, `skills`, `theme`, `vim`, `voice`, …

---

## 3. Hooks (`src/hooks/` + `src/query/stopHooks.ts` ↔ `claude-core/src/hooks/`)

**Hook event taxonomy: full parity (27/27 types)** — PreToolUse, PostToolUse,
PostToolUseFailure, Stop, StopFailure, SubagentStart, SubagentStop,
SessionStart, SessionEnd, UserPromptSubmit, PreCompact, PostCompact,
Notification, PermissionRequest, PermissionDenied, Setup, TeammateIdle,
TaskCreated, TaskCompleted, Elicitation, ElicitationResult, ConfigChange,
WorktreeCreate, WorktreeRemove, InstructionsLoaded, CwdChanged, FileChanged.

Rust has `types.rs`, `runner.rs`, `matching.rs`, `aggregation.rs`,
`tool_hooks.rs`, `ssrf.rs` — execution framework is present but **dispatch
from tool/command code is thin** (e.g. `task_tools.rs` does not fire
`TaskCreated`, query engine does not fire `StopFailure`). TS `src/hooks/` is
mostly React UI hooks (N/A for Rust).

---

## 4. Query engine (`query.ts` + `QueryEngine.ts` ↔ `claude-core/src/query/`)

TS: ~3024 LOC (`query.ts` 1729, `QueryEngine.ts` 1295, plus config/tokenBudget/stopHooks/deps).
Rust: `engine.rs` 780, `tool_executor.rs` 166, `state.rs` 34, `mod.rs`.

### Missing in Rust
- **Microcompact** with cache-edit of pending content (TS ~lines 414–425)
- **Context collapse** feature (TS ~lines 440–447)
- **Attachment + relevant-memory prefetch** (`getAttachmentMessages`, `startRelevantMemoryPrefetch`)
- **Tool-result budget/truncation** (`applyToolResultBudget`)
- **Stop-failure hook execution** (`executeStopFailureHooks`)
- **Tool-use summarization** (`generateToolUseSummary`)
- **File-history snapshot** integration during queries
- **Structured output enforcement** (`registerStructuredOutputEnforcement`)
- **Full permission-request flow during tool use**
- **Detailed analytics** — TS logs 10+ event types; Rust uses minimal `tracing::warn!`

### Present in Rust
SSE streaming, reactive compact on prompt-too-long, max-token escalation,
cancellation, basic user-context build, `repair_tool_use_results` (lighter
than TS `normalizeMessagesForAPI`).

---

## 5. Services (`src/services/` ↔ various `claude-core/src/*`)

| TS service | LOC | Rust | Status |
|---|---|---|---|
| `analytics/` | ~1.1K | `analytics/` | present, thin |
| `api/` | **~10.5K (21 files)** | `api/` ~1.2K (6 files) | **~90% thin** — missing filesApi, grove, sessionIngress, ultrareviewQuota, overageCreditGrant, firstTokenDate, referral, adminRequests, bootstrap, metricsOptOut, promptCacheBreakDetection, emptyUsage, logging depth |
| `autoDream/` | ~500 | — | **MISSING** |
| `compact/` | 11 files | `compact/` ~965 LOC | partial — missing sessionMemoryCompact, apiMicrocompact, microCompact, timeBasedMCConfig, postCompactCleanup, compactWarningHook, reactiveCompact details |
| `extractMemories/` | ~200 | — | **MISSING** |
| `lsp/` | ~1.1K | `lsp/` ~51K | Rust-heavy; present |
| `MagicDocs/` | ~300 | — | **MISSING** |
| `mcp/` | ~2.5K (25 files) | `mcp/` ~1.9K (4 files) | **~90% thin** — no OAuth, no in-process transport, no official registry, no xaa IdP |
| `oauth/` | ~800 | `auth/` ~900 | present |
| `plugins/` | ~600 | `plugins/` ~450 | thinner — no built-in registry, no UI |
| `policyLimits/` | ~150 | — | **MISSING** |
| `PromptSuggestion/` | ~500 | — | **MISSING** |
| `remoteManagedSettings/` | ~200 | — | **MISSING** |
| `SessionMemory/` | ~1K | — | **MISSING** |
| `settingsSync/` | ~150 | — | **MISSING** |
| `teamMemorySync/` | ~600 | partial under `teams/` | partial |
| `tips/` | ~400 | — | **MISSING** |
| `tools/` | **~3.1K (streaming executor)** | `query/tool_executor.rs` 166 LOC | **~95% thin** — no streaming tool executor |
| `toolUseSummary/` | ~200 | — | **MISSING** |
| `AgentSummary/` | ~300 | — | **MISSING** |

---

## 6. UI — components / screens / ink / dialogs

TS: ~389 components + 3 screens + Ink renderer + Yoga layout (~150 K+ LOC).
Rust: ~16 widgets under `claude-tui/src/widgets/` on ratatui.

### Widgets present
`model_picker`, `onboarding`, `permission_dialog`, `trust_dialog`,
`compact_summary`, `message_list`, `prompt_input`, `task_list`, `diff_view`
(basic), `feedback_dialog` (basic).

### Entirely missing dialogs / panels
- **MCP**: `MCPServerApprovalDialog`, `MCPReconnect`, `McpParsingWarnings`,
  `CapabilitiesSection`, `MCPToolListView`, `MCPToolDetailView`,
  `MCPSettings`, server menus, `MCPListPanel`, `ElicitationDialog`
- **IDE / onboarding**: `ClaudeInChromeOnboarding`, `IdeOnboardingDialog`,
  `IdeAutoConnectDialog`, `BridgeDialog`
- **Session / data**: `ExportDialog`, `WorktreeExitDialog`,
  `IdleReturnDialog`, `AutoModeOptInDialog`, `ResumeConversation` UI
- **Config / errors**: `InvalidConfigDialog`, `InvalidSettingsDialog`,
  `CostThresholdDialog`, `LanguagePicker`
- **Context**: `ContextSuggestions`, `ContextVisualization`
- **Diagnostic**: `Doctor` screen, system health panel, LSP-status visibility
- **Nav / search**: global search (`app:globalSearch`), quick-open
  (`app:quickOpen`), history search
- **File edits**: `FileEditToolDiff`, `FileEditToolUpdatedMessage`,
  structured-diff rendering
- **Infrastructure**: `dialogLaunchers.tsx`, `interactiveHelpers.tsx`

---

## 7. Context / memory / migrations

### Context — partial
Rust `context/` has `environment.rs`, `git.rs`, `system_prompt.rs`.
Missing: CLAUDE.md aggregation, memory-file handling, full
`getSystemContext` / `getUserContext` pipelines.

### Memory (`memdir/`) — **entirely missing in Rust**
TS has `memdir.ts`, `findRelevantMemories.ts`, `memoryScan.ts`,
`memoryAge.ts`, `memoryTypes.ts`, MEMORY.md loader (200-line / 25 KB
truncation), team-memory paths, auto-memory detection. No Rust equivalent.

### Migrations — **effectively missing**
Rust `claude-core/src/migrations/` is empty. TS has 11 migration files:
auto-updates, bypass permissions, MCP enable, fennec→opus, legacy opus,
opus→opus1m, repl-bridge→remote-control, sonnet 1m→45→46, auto-mode reset,
pro→opus defaults.

---

## 8. Bridge / server / remote / upstream proxy / entrypoints

### Bridge — **~95% thin**
TS `src/bridge/` = 31 files, ~12.6 K LOC (`bridgeMain` 113 KB, `replBridge`
98 KB, `remoteBridgeCore` 39 KB, plus messaging, initialization, config,
security, UI). Rust `bridge/` = 4 files (`server.rs` 9 KB, `protocol.rs` 2 KB,
`types.rs` 3 KB). No `bridgeMessaging`, `initReplBridge`, `sessionRunner`,
JWT/workSecret, trusted-device, capacityWake, flushGate, pollConfig, bridge UI,
permission callbacks.

### Server (direct connection) — **entirely missing**
TS `src/server/directConnectManager.ts` (WebSocket session manager) has no
Rust equivalent.

### Remote — **~75% thin**
Rust `remote/client.rs` (2.8 KB) has only task create/status/cancel + env
register. TS `RemoteSessionManager` is far more comprehensive.

### Upstream proxy — **entirely missing**
TS `src/upstreamproxy/{upstreamproxy.ts, relay.ts}` (~24.7 K LOC) has no Rust
equivalent. Rust `proxy/` is a local HTTP client, not the relay.

### Entrypoints / CLI
TS `src/entrypoints/cli.tsx` 39 KB + SDK schemas (`coreSchemas` 1.9 K LOC,
`controlSchemas` 663 LOC) + `init.ts` + `agentSdkTypes` + `sandboxTypes` +
`mcp.ts`. Rust `claude-cli/src/main.rs` is a thin binary entry.
**SDK schemas and init orchestration not ported.**

---

## 9. MCP

TS total: ~20.7 K LOC across `services/mcp/` (25 files), `utils/mcp/`,
`components/mcp/` (12 React components), `commands/mcp/`, `entrypoints/mcp.ts`,
`skills/mcpSkillBuilders.ts`, plus WebSocket transport and computerUse /
Chrome MCP servers.

Rust `mcp/` = 4 files, ~1.9 K LOC — stdio, SSE, HTTP transports, JSON-RPC,
manager with tool discovery.

### Missing in Rust
WebSocket transport, reconnection/fallback logic, elicitation dialog, parsing
warnings UI, OAuth (`xaaIdpCommand`), in-process transport, official registry,
`mcpInstructionsDelta`, `mcpOutputStorage`, `elicitationValidation`, skill
builders (`mcpSkillBuilders.ts`).

---

## 10. Bootstrap / setup / constants / schemas / types

### Bootstrap / global state — **missing**
TS `src/bootstrap/state.ts` is a 1.8 K LOC global state object (cost, session,
model usage, hooks, settings cache, OpenTelemetry attrs/meters). Rust
`claude-core/src/bootstrap/` is effectively empty.

### Constants — **~95% reduction**
TS `src/constants/` = 21 files (`prompts.ts` 54 KB, `outputStyles` 10 KB,
`oauth` 9 KB, `github-app` 5 KB, `apiLimits` 3 KB, plus system, figures,
files, keys, product, spinnerVerbs, systemPromptSections, toolLimits, tools,
turnCompletionVerbs, xml, betas, common, cyberRiskInstruction, errorIds,
messages). Rust `constants/` = 2 files (`api_limits.rs`, `files.rs`).
**The prompts database is effectively absent.**

### Schemas — missing
TS `src/schemas/hooks.ts` (7.9 KB) has no Rust counterpart.

### Types — thin
Rust `types/` = 6 files (~6 K LOC). TS has dozens of type modules.

---

## 11. Other top-level TS subsystems

| TS | Rust | Status |
|---|---|---|
| `keybindings/` (~14 files) | — | **MISSING** — no user-customizable bindings |
| `vim/` (6 files: motions, operators, transitions, textObjects) | — | **MISSING** |
| `voice/` (7 files, GrowthBook kill-switch, Anthropic OAuth) | `voice/` (recorder, transcriber, types) | partial — recorder/transcriber exist; auth gating + Claude.ai voice endpoint missing |
| `buddy/` (companion sprite/mascot) | — | **MISSING** |
| `coordinator/coordinatorMode.ts` (370-line system prompt) | `teams/coordinator.rs` | partial — mode detection exists; prompt assembly + worker tool restrictions missing |
| `tasks/` (LocalShellTask, LocalAgentTask, RemoteAgentTask, DreamTask, LocalWorkflowTask, MonitorMcpTask, InProcessTeammateTask) | `task_tools.rs` + `teams/spawn.rs` | partial — no DreamTask, RemoteAgentTask, LocalWorkflowTask, MonitorMcpTask |
| `outputStyles/` | — | **MISSING** |
| `plugins/` (50+ files, built-in registry) | `plugins/` (5 files) | partial — no built-in registry, no UI, no marketplace flow |
| `assistant/sessionHistory.ts` | `session/` | partial |
| `native-ts/` (color-diff 30 KB, file-index 12 KB, yoga-layout 83 KB) | — | **MISSING** (yoga unneeded; color-diff + file-index are logic gaps) |
| `moreright/useMoreRight.tsx` | — | N/A (React layout hook) |
| `cost-tracker.ts` + `costHook.ts` (~11 KB) | `cost/tracker.rs` (~3 KB) | thin — no cost hook integration |
| `history.ts` | `session/storage.rs` | partial |

---

## 12. Rust-only subsystems (not in TS)

| Rust | LOC | Purpose |
|---|---|---|
| `teams/` | ~192 K (backends, coordinator, mailbox, memory, permission_sync, spawn, types) | multi-agent coordination |
| `sandbox/` | ~32 K (executor, tracker) | OS-level isolation for tool execution |
| `lsp/` | ~51 K (client, manager, types) | full LSP client (richer than TS `services/lsp`) |
| `vcr/` | ~8 K (player, recorder) | record/replay HTTP for tests |
| `workflows/` | ~12 K (runner) | workflow execution engine |
| `file_history/` | ~3 K (tracker) | file change tracking (primitive — not yet wired into Edit/Write) |
| `compact/` | ~30 K | deeper compaction mechanics (but missing microcompact) |

---

## 13. Prioritized parity checklist

### P0 — shipping blockers
1. **WebFetch prompt parameter** — currently silently ignored.
2. **FileEdit / FileWrite staleness check + git diff output** — correctness.
3. **Tool registry feature-flag gating** — flag-gated tools always on in Rust.
4. **Global state store** (`bootstrap/state`) — cross-cutting (cost, telemetry, session).
5. **Prompts database** (`constants/prompts.ts`) — no amount of code compensates for missing prompts.
6. **Migrations subsystem** — empty directory; settings/model migrations don't run.
7. **Command handlers** — 29 fully missing, ~43 stub-only; e.g. `login`, `logout`, `mcp`, `config`, `permissions`, `memory`, `resume`.
8. **Memory system (`memdir/`)** — entirely absent.

### P1 — feature parity
9. **Bridge / upstream proxy** — missing replBridge, messaging, relay.
10. **Services/api depth** — filesApi, grove, sessionIngress, adminRequests, caching.
11. **Query engine extras** — microcompact cache edit, context collapse, attachment prefetch, tool-result budgeting, stop-failure hooks.
12. **MCP UI + OAuth + WebSocket transport + skill builders.**
13. **Task hooks + external persistence + activeForm + owner/team fields.**
14. **BashTool permission matrix + sed parser/validator.**
15. **AgentTool fork mode.**

### P2 — UX polish
16. Keybindings system + vim mode.
17. Output styles.
18. Specialized dialogs (MCP approval, IDE picker, cost threshold, invalid-config, export, worktree-exit).
19. Plugin UI + built-in plugin registry.
20. Session memory, autoDream, tips, PromptSuggestion, MagicDocs, extractMemories.

### Non-goals
- `buddy/` mascot system.
- Yoga layout engine (ratatui handles layout).
- Ink terminal renderer (ratatui is the Rust equivalent).
- React component library (N/A for ratatui).

---

## 14. Key reference files for porting

- `src/tools/WebFetchTool/utils.ts` — `applyPromptToMarkdown`
- `src/tools/BashTool/bashPermissions.ts` (96 KB) — command permission matrix
- `src/tools/FileEditTool/FileEditTool.ts` lines 80–200 — staleness + git diff
- `src/tools/TaskCreateTool/TaskCreateTool.ts` lines 93–130 — hook execution
- `src/tools.ts` lines 14–135 — feature-gate patterns for the registry
- `src/bootstrap/state.ts` — global-state shape to mirror
- `src/constants/prompts.ts` — the prompts database
- `src/query/query.ts` lines 400–480 — microcompact + context collapse
- `src/bridge/bridgeMain.ts` + `replBridge.ts` — bridge architecture
- `src/memdir/` — full memory subsystem
- `src/migrations/*.ts` — migration patterns
