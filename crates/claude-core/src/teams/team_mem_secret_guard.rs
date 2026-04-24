//! Block secret leaks into team memory files.
//!
//! Port of TS `src/services/teamMemorySync/teamMemSecretGuard.ts`.
//! Called from `FileWriteTool` / `FileEditTool` validateInput
//! paths to prevent the model from writing API keys, tokens, or
//! similar credentials into team memory files that would then sync
//! to every repository collaborator.
//!
//! Gate behaviour (matches TS):
//! - When the `TEAMMEM` feature flag is off, always returns `None`
//!   (no validation — feature is dormant).
//! - When the path isn't a team memory path, returns `None`
//!   (the validation only applies to team memory).
//! - When secrets are detected, returns the error message to
//!   surface through the tool's validateInput response.

use crate::errors_util::is_env_truthy;
use crate::memdir::team_mem_paths::is_team_mem_path;
use crate::secret_scanner::scan_for_secrets;
use std::path::Path;

/// Check if a file write/edit to a team memory path contains
/// secrets. Returns `Some(error_message)` if:
/// 1. The `TEAMMEM` feature flag is truthy (matches TS
///    `feature('TEAMMEM')` at `teamMemSecretGuard.ts:19`).
/// 2. The path is inside the team memory directory for `cwd`.
/// 3. The content contains one or more secret patterns from the
///    shared `secret_scanner`.
///
/// Returns `None` when any check fails.
///
/// Gate note: this does NOT call `is_team_memory_enabled()`.
/// Codex CR caught that the earlier implementation added an
/// extra `auto_memory_enabled()` check TS doesn't have — TS
/// guards only on the build flag and scans any write to a
/// team-memory path when the flag is on, whether or not the
/// broader "team memory" user feature is active. Keeping that
/// shape is important for parity: the guard's job is to prevent
/// secret leaks in ANY team-memory write the tool might produce.
///
/// The error message lists the labels of the matched secret
/// kinds so the user can tell what was flagged without the
/// message quoting the actual secret value — matches TS's
/// `labels = matches.map(m => m.label).join(', ')` shape.
pub fn check_team_mem_secrets(file_path: &Path, content: &str, cwd: &Path) -> Option<String> {
    if !is_env_truthy("TEAMMEM") {
        return None;
    }
    if !is_team_mem_path(file_path, cwd) {
        return None;
    }
    let matches = scan_for_secrets(content);
    if matches.is_empty() {
        return None;
    }
    let labels: Vec<String> = matches.iter().map(|m| m.label.clone()).collect();
    Some(format!(
        "Content contains potential secrets ({}) and cannot be written to team memory. \
         Team memory is shared with all repository collaborators. \
         Remove the sensitive content and try again.",
        labels.join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::ENV_LOCK;
    use crate::memdir::team_mem_paths::get_team_mem_path;
    use std::path::Path;

    /// Known secret pattern that `secret_scanner` recognises —
    /// taken from the scanner's unit tests so this module doesn't
    /// duplicate the sensitive-prefix knowledge.
    const AWS_KEY: &str = "AKIAIOSFODNN7EXAMPLE";

    fn enable_team_mem(lock: &std::sync::MutexGuard<()>) {
        let _ = lock; // bind for lifetime; caller holds the ENV_LOCK.
        std::env::remove_var("CLAUDE_CODE_DISABLE_AUTO_MEMORY");
        std::env::set_var("TEAMMEM", "1");
    }

    fn disable_team_mem() {
        std::env::remove_var("TEAMMEM");
    }

    #[test]
    fn returns_none_when_feature_off() {
        let g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        disable_team_mem();
        let cwd = Path::new("/Users/alex/proj");
        let path = get_team_mem_path(cwd).join("file.md");
        let out = check_team_mem_secrets(&path, AWS_KEY, cwd);
        assert!(out.is_none());
        drop(g);
    }

    #[test]
    fn returns_none_when_path_outside_team_dir() {
        let g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        enable_team_mem(&g);
        let cwd = Path::new("/Users/alex/proj");
        let not_team = Path::new("/Users/alex/proj/src/main.rs");
        let out = check_team_mem_secrets(not_team, AWS_KEY, cwd);
        assert!(out.is_none(), "non-team path must not trigger the guard");
        disable_team_mem();
        drop(g);
    }

    #[test]
    fn returns_none_when_content_has_no_secrets() {
        let g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        enable_team_mem(&g);
        let cwd = Path::new("/Users/alex/proj");
        let path = get_team_mem_path(cwd).join("benign.md");
        let out = check_team_mem_secrets(
            &path,
            "Just regular team memory content — no keys here.",
            cwd,
        );
        assert!(out.is_none());
        disable_team_mem();
        drop(g);
    }

    #[test]
    fn returns_error_when_secret_in_team_path() {
        let g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        enable_team_mem(&g);
        let cwd = Path::new("/Users/alex/proj");
        let path = get_team_mem_path(cwd).join("leaked.md");
        let out = check_team_mem_secrets(&path, AWS_KEY, cwd);
        let msg = out.expect("must flag secret");
        assert!(msg.contains("potential secrets"));
        assert!(msg.contains("Team memory is shared"));
        assert!(msg.contains("Remove the sensitive content"));
        disable_team_mem();
        drop(g);
    }

    #[test]
    fn error_message_lists_secret_labels() {
        let g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        enable_team_mem(&g);
        let cwd = Path::new("/Users/alex/proj");
        let path = get_team_mem_path(cwd).join("leaked.md");
        let out = check_team_mem_secrets(&path, AWS_KEY, cwd).expect("must flag secret");
        // scan_for_secrets returns entries with non-empty labels;
        // the guard message should splice them between the parens.
        let open = out.find('(').expect("label-open paren");
        let close = out.find(')').expect("label-close paren");
        assert!(close > open);
        let labels_blob = &out[open + 1..close];
        assert!(!labels_blob.is_empty(), "label list must be non-empty");
        disable_team_mem();
        drop(g);
    }

    /// Exact-string parity with TS — caught-by-codex request:
    /// the guard's full message template must match
    /// `teamMemSecretGuard.ts:38-41` byte-for-byte modulo the
    /// single-space line-continuation collapse.
    #[test]
    fn error_message_matches_ts_exactly() {
        let g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        enable_team_mem(&g);
        let cwd = Path::new("/Users/alex/proj");
        let path = get_team_mem_path(cwd).join("leaked.md");
        let out = check_team_mem_secrets(&path, AWS_KEY, cwd).expect("must flag secret");
        // Message shape: prefix, labels-in-parens, two suffix sentences.
        let scan = crate::secret_scanner::scan_for_secrets(AWS_KEY);
        let labels: Vec<String> = scan.iter().map(|m| m.label.clone()).collect();
        let expected = format!(
            "Content contains potential secrets ({}) and cannot be written to team memory. Team memory is shared with all repository collaborators. Remove the sensitive content and try again.",
            labels.join(", ")
        );
        assert_eq!(out, expected);
        disable_team_mem();
        drop(g);
    }

    /// Codex CR: the earlier Rust guard bailed out when
    /// `auto_memory_enabled()` was off, which TS does NOT do — TS
    /// guards only on the `feature('TEAMMEM')` build flag. This
    /// test pins the parity fix: with TEAMMEM on + auto-memory
    /// DISABLED, the guard still fires when a secret lands in a
    /// team-memory path.
    #[test]
    fn scans_even_when_auto_memory_disabled() {
        let g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        std::env::set_var("TEAMMEM", "1");
        std::env::set_var("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "true");
        let cwd = Path::new("/Users/alex/proj");
        let path = get_team_mem_path(cwd).join("leaked.md");
        let out = check_team_mem_secrets(&path, AWS_KEY, cwd);
        assert!(
            out.is_some(),
            "guard must scan team-path writes when TEAMMEM is on, even if auto-memory is off"
        );
        std::env::remove_var("CLAUDE_CODE_DISABLE_AUTO_MEMORY");
        std::env::remove_var("TEAMMEM");
        drop(g);
    }
}
