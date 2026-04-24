//! Memory directory subsystem.
//!
//! Ports the stateless/data-shape pieces of `src/memdir/` from TS:
//!   - `memoryTypes.ts` → `types`        (MemoryType enum + parse)
//!   - `memoryAge.ts`   → `age`          (age math + freshness text)
//!   - `memoryScan.ts`  → `scan`         (directory scan + frontmatter headers)
//!   - `memdir.ts`      → `entrypoint`   (MEMORY.md truncation)
//!   - `paths.ts`       → `paths`        (auto-memory directory resolution)
//!
//! Not ported (TS-specific or out-of-scope for this session):
//!   - TYPES_SECTION_COMBINED / TYPES_SECTION_INDIVIDUAL / prompt builders —
//!     large static prompt strings that belong in the prompts database, not
//!     the logic module. `memoryTypes.ts` alone is ~600 lines of prompt text.
//!   - Team memory (`teamMemPaths`, `teamMemPrompts`) — gated on feature
//!     `TEAMMEM`, and pulls in the team-coordinator surface.
//!   - Kairos daily-log path (`getAutoMemDailyLogPath`) — gated on feature
//!     `KAIROS`.
//!   - Memoization via `getProjectRoot()`, settings-based override chain,
//!     `findCanonicalGitRoot`, `CLAUDE_COWORK_MEMORY_PATH_OVERRIDE` —
//!     depend on bootstrap/state and settings sources the Rust side still
//!     wires differently. A simple env-var + HOME fallback is ported here.

pub mod age;
pub mod daily_log_prompt;
pub mod entrypoint;
pub mod paths;
pub mod prompt;
pub mod scan;
pub mod searching_past_context;
pub mod team_mem_paths;
pub mod types;

pub use age::{memory_age, memory_age_days, memory_freshness_note, memory_freshness_text};
pub use daily_log_prompt::{build_assistant_daily_log_prompt, DailyLogPromptInputs};
pub use entrypoint::{
    truncate_entrypoint_content, EntrypointTruncation, MAX_ENTRYPOINT_BYTES, MAX_ENTRYPOINT_LINES,
};
pub use paths::{
    auto_memory_enabled, get_auto_mem_entrypoint, get_auto_mem_path, get_memory_base_dir,
};
pub use prompt::{
    build_memory_lines, memory_frontmatter_example, MEMORY_DRIFT_CAVEAT, TRUSTING_RECALL_SECTION,
    TYPES_SECTION_INDIVIDUAL, WHAT_NOT_TO_SAVE_SECTION, WHEN_TO_ACCESS_SECTION,
};
pub use scan::{format_memory_manifest, scan_memory_files, MemoryHeader};
pub use types::{parse_memory_type, MemoryType, MEMORY_TYPES};
