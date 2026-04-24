//! Auto-dream (memory consolidation) helpers.
//!
//! Port of `src/services/autoDream/{config,consolidationPrompt,consolidationLock}.ts`.
//! TS is 550 LOC total — the full dream path ships a forked agent
//! + consolidation-lock machinery to prevent concurrent dreams across
//!   sessions. This module ports the self-contained pieces a Rust
//!   caller can use today:
//!
//!   - `is_auto_dream_enabled()` — env/settings-driven gate (matches
//!     TS `isAutoDreamEnabled`; the GrowthBook fallback tier is
//!     replaced by an env var since we haven't ported GrowthBook).
//!   - `build_consolidation_prompt(memory_root, transcript_dir, extra)`
//!     verbatim from TS `buildConsolidationPrompt`.
//!   - `HOLDER_STALE_MS` / `LOCK_FILE` constants so a future port of
//!     the lock layer uses matching names.
//!
//! The forked-agent dream machinery + lock acquire/release/rollback is
//! deferred — see services/AgentSummary module doc for the same
//! deferred-forked-agent story.

use crate::memdir::entrypoint::{ENTRYPOINT_NAME, MAX_ENTRYPOINT_LINES};

/// Env var override for `autoDreamEnabled`. Setting this to a truthy
/// value enables consolidation without needing a settings.json entry
/// or GrowthBook access.
pub const AUTO_DREAM_ENABLED_ENV: &str = "CLAUDE_CODE_AUTO_DREAM";

/// Lock file name inside the memory dir. Matches TS.
pub const LOCK_FILE: &str = ".consolidate-lock";

/// Time after which a lock is considered stale even if the holder PID
/// is still alive. Prevents a crashed+pid-reused process from blocking
/// forever. Matches TS `HOLDER_STALE_MS = 60 * 60 * 1000`.
pub const HOLDER_STALE_MS: u64 = 60 * 60 * 1000;

/// Env-driven autoDream gate. Env var takes precedence; absent, falls
/// back to the `CLAUDE_CODE_SIMPLE` inverse (simple mode disables
/// dreams). The TS GrowthBook fallback (`tengu_onyx_plover`) isn't
/// wired in Rust — callers with explicit settings should pass them
/// in rather than rely on this default.
pub fn is_auto_dream_enabled() -> bool {
    fn truthy(v: &str) -> bool {
        matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on")
    }
    fn falsy(v: &str) -> bool {
        matches!(
            v.to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        )
    }

    if let Ok(v) = std::env::var(AUTO_DREAM_ENABLED_ENV) {
        if truthy(&v) {
            return true;
        }
        if falsy(&v) {
            return false;
        }
    }
    // --bare / SIMPLE disables background work.
    !std::env::var("CLAUDE_CODE_SIMPLE")
        .map(|v| truthy(&v))
        .unwrap_or(false)
}

/// DIR_EXISTS_GUIDANCE — same text memdir uses so the prompt says
/// "directory already exists" consistently. Pulling from a shared
/// string here rather than re-emitting would require memdir to expose
/// the constant; re-emitting verbatim matches TS and keeps the prompt
/// byte-identical.
const DIR_EXISTS_GUIDANCE: &str =
    "This directory already exists — write to it directly with the Write tool (do not run mkdir or check for its existence).";

/// Build the consolidation prompt sent to the forked dream agent.
/// Verbatim port of TS `buildConsolidationPrompt`. Text is reproduced
/// character-for-character so prompt-cache prefix-matches across
/// implementations. `extra` is appended under "Additional context"
/// when non-empty; pass "" to omit.
pub fn build_consolidation_prompt(memory_root: &str, transcript_dir: &str, extra: &str) -> String {
    let mut out = String::new();
    out.push_str("# Dream: Memory Consolidation\n\n");
    out.push_str("You are performing a dream — a reflective pass over your memory files. Synthesize what you've learned recently into durable, well-organized memories so that future sessions can orient quickly.\n\n");
    out.push_str(&format!("Memory directory: `{}`\n", memory_root));
    out.push_str(DIR_EXISTS_GUIDANCE);
    out.push_str("\n\n");
    out.push_str(&format!(
        "Session transcripts: `{}` (large JSONL files — grep narrowly, don't read whole files)\n\n",
        transcript_dir
    ));
    out.push_str("---\n\n");
    out.push_str("## Phase 1 — Orient\n\n");
    out.push_str("- `ls` the memory directory to see what already exists\n");
    out.push_str(&format!(
        "- Read `{}` to understand the current index\n",
        ENTRYPOINT_NAME
    ));
    out.push_str(
        "- Skim existing topic files so you improve them rather than creating duplicates\n",
    );
    out.push_str("- If `logs/` or `sessions/` subdirectories exist (assistant-mode layout), review recent entries there\n\n");
    out.push_str("## Phase 2 — Gather recent signal\n\n");
    out.push_str("Look for new information worth persisting. Sources in rough priority order:\n\n");
    out.push_str("1. **Daily logs** (`logs/YYYY/MM/YYYY-MM-DD.md`) if present — these are the append-only stream\n");
    out.push_str("2. **Existing memories that drifted** — facts that contradict something you see in the codebase now\n");
    out.push_str("3. **Transcript search** — if you need specific context (e.g., \"what was the error message from yesterday's build failure?\"), grep the JSONL transcripts for narrow terms:\n");
    out.push_str(&format!(
        "   `grep -rn \"<narrow term>\" {}/ --include=\"*.jsonl\" | tail -50`\n\n",
        transcript_dir
    ));
    out.push_str(
        "Don't exhaustively read transcripts. Look only for things you already suspect matter.\n\n",
    );
    out.push_str("## Phase 3 — Consolidate\n\n");
    out.push_str("For each thing worth remembering, write or update a memory file at the top level of the memory directory. Use the memory file format and type conventions from your system prompt's auto-memory section — it's the source of truth for what to save, how to structure it, and what NOT to save.\n\n");
    out.push_str("Focus on:\n");
    out.push_str(
        "- Merging new signal into existing topic files rather than creating near-duplicates\n",
    );
    out.push_str("- Converting relative dates (\"yesterday\", \"last week\") to absolute dates so they remain interpretable after time passes\n");
    out.push_str("- Deleting contradicted facts — if today's investigation disproves an old memory, fix it at the source\n\n");
    out.push_str("## Phase 4 — Prune and index\n\n");
    out.push_str(&format!(
        "Update `{}` so it stays under {} lines AND under ~25KB. It's an **index**, not a dump — each entry should be one line under ~150 characters: `- [Title](file.md) — one-line hook`. Never write memory content directly into it.\n\n",
        ENTRYPOINT_NAME, MAX_ENTRYPOINT_LINES
    ));
    out.push_str("- Remove pointers to memories that are now stale, wrong, or superseded\n");
    out.push_str("- Demote verbose entries: if an index line is over ~200 chars, it's carrying content that belongs in the topic file — shorten the line, move the detail\n");
    out.push_str("- Add pointers to newly important memories\n");
    out.push_str("- Resolve contradictions — if two files disagree, fix the wrong one\n\n");
    out.push_str("---\n\n");
    out.push_str("Return a brief summary of what you consolidated, updated, or pruned. If nothing changed (memories are already tight), say so.");
    if !extra.is_empty() {
        out.push_str("\n\n## Additional context\n\n");
        out.push_str(extra);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // autoDream tests mutate env vars; serialise.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn disabled_when_simple_mode() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var(AUTO_DREAM_ENABLED_ENV);
        std::env::set_var("CLAUDE_CODE_SIMPLE", "1");
        assert!(!is_auto_dream_enabled());
        std::env::remove_var("CLAUDE_CODE_SIMPLE");
    }

    #[test]
    fn env_override_enables() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("CLAUDE_CODE_SIMPLE", "1");
        std::env::set_var(AUTO_DREAM_ENABLED_ENV, "true");
        assert!(is_auto_dream_enabled());
        std::env::remove_var(AUTO_DREAM_ENABLED_ENV);
        std::env::remove_var("CLAUDE_CODE_SIMPLE");
    }

    #[test]
    fn env_override_disables() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("CLAUDE_CODE_SIMPLE");
        std::env::set_var(AUTO_DREAM_ENABLED_ENV, "false");
        assert!(!is_auto_dream_enabled());
        std::env::remove_var(AUTO_DREAM_ENABLED_ENV);
    }

    #[test]
    fn prompt_has_four_phases() {
        let p = build_consolidation_prompt("/tmp/mem", "/tmp/proj", "");
        assert!(p.contains("## Phase 1 — Orient"));
        assert!(p.contains("## Phase 2 — Gather recent signal"));
        assert!(p.contains("## Phase 3 — Consolidate"));
        assert!(p.contains("## Phase 4 — Prune and index"));
    }

    #[test]
    fn prompt_inlines_memdir_and_transcript() {
        let p = build_consolidation_prompt("/a/memory", "/b/transcripts", "");
        assert!(p.contains("`/a/memory`"));
        assert!(p.contains("`/b/transcripts`"));
    }

    #[test]
    fn prompt_appends_extra_context_when_non_empty() {
        let p = build_consolidation_prompt("/a", "/b", "user added this hint");
        assert!(p.contains("## Additional context"));
        assert!(p.contains("user added this hint"));
    }

    #[test]
    fn prompt_skips_extra_context_when_empty() {
        let p = build_consolidation_prompt("/a", "/b", "");
        assert!(!p.contains("## Additional context"));
    }

    #[test]
    fn stale_window_matches_ts() {
        // TS: 60 * 60 * 1000 = 3600000
        assert_eq!(HOLDER_STALE_MS, 3_600_000);
    }
}
