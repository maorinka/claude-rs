//! Detection for the "embedded search tools" build mode.
//!
//! Port of TS `utils/embeddedTools.ts:1-29`.
//!
//! Background (from TS file): some distributions of Claude Code ship with
//! `bfs` and `ugrep` embedded directly in the bundled Bun binary (the
//! "ant-native" build). In that mode the Bash shell shadows `find` and
//! `grep` with shell functions that re-invoke the bundled binary with a
//! spoofed `argv0`, the dedicated Glob/Grep tools are dropped from the
//! registry, and the model-facing prompt guidance changes.
//!
//! Rust doesn't ship a native-with-embedded-search build right now, so
//! [`has_embedded_search_tools`] currently always returns `false`. The
//! function is kept with the same gating surface (env var +
//! entrypoint-kind check) so if a native Rust build ever acquires the
//! same capability, only the constant `EMBEDDED_BUILD` needs flipping.

use crate::errors_util::is_env_truthy;

/// Compile-time flag for whether this build actually bundles `bfs` / `ugrep`.
/// False for every published Rust build today. TS sets the equivalent as a
/// build-time define in `scripts/build-with-plugins.ts`; Rust can flip this
/// when an analogous native bundle lands.
const EMBEDDED_BUILD: bool = false;

/// SDK / local-agent entrypoints that suppress the shell shadowing even
/// when the build would otherwise have embedded tools. Matches TS
/// `embeddedTools.ts:19` — the four entrypoints there are `sdk-ts`,
/// `sdk-py`, `sdk-cli`, `local-agent`.
const NON_EMBEDDED_ENTRYPOINTS: &[&str] = &["sdk-ts", "sdk-py", "sdk-cli", "local-agent"];

/// Returns `true` when this build has `bfs`/`ugrep` embedded and the
/// current entrypoint is one that should use them. Gated on both the
/// `EMBEDDED_SEARCH_TOOLS` env var (so turnkey disable is possible) and
/// the `CLAUDE_CODE_ENTRYPOINT` value (so SDK / local-agent invocations
/// keep using the standalone Glob/Grep tools).
pub fn has_embedded_search_tools() -> bool {
    if !EMBEDDED_BUILD {
        return false;
    }
    if !is_env_truthy("EMBEDDED_SEARCH_TOOLS") {
        return false;
    }
    match std::env::var("CLAUDE_CODE_ENTRYPOINT") {
        Ok(e) => !NON_EMBEDDED_ENTRYPOINTS.contains(&e.as_str()),
        // Absent entrypoint = not one of the SDK/local-agent values, so the
        // embedded tools apply. TS's `e !== 'sdk-ts' && …` with `e === undefined`
        // evaluates truthy the same way.
        Err(_) => true,
    }
}

/// Path to the binary that hosts the embedded tools. TS returns
/// `process.execPath` — the Bun binary. Rust's closest equivalent is
/// [`std::env::current_exe`], which resolves the running binary's path.
///
/// Only meaningful when [`has_embedded_search_tools`] returns `true`.
/// Returns `None` if the current-exe lookup fails (symlink that no
/// longer resolves, sandboxed mount, etc.).
pub fn embedded_search_tools_binary_path() -> Option<std::path::PathBuf> {
    std::env::current_exe().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_embedded_search_tools_is_false_in_default_rust_build() {
        // `EMBEDDED_BUILD` is `false`, so every combination of env vars must
        // short-circuit to false — no env mutation needed for this assertion.
        assert!(!has_embedded_search_tools());
    }

    #[test]
    fn non_embedded_entrypoint_list_matches_ts() {
        // Guards against accidental drift — TS lists exactly these four.
        assert_eq!(
            NON_EMBEDDED_ENTRYPOINTS,
            &["sdk-ts", "sdk-py", "sdk-cli", "local-agent"]
        );
    }

    #[test]
    fn binary_path_resolves_for_test_binary() {
        // `cargo test` runs each test inside a binary; `current_exe` must
        // succeed there.
        assert!(embedded_search_tools_binary_path().is_some());
    }
}
