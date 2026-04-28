/// Return whether git-specific prompt/context should be included.
///
/// Mirrors TS `src/utils/gitSettings.ts`:
/// - `CLAUDE_CODE_DISABLE_GIT_INSTRUCTIONS` truthy disables.
/// - the same env var explicitly falsy enables.
/// - otherwise fall back to merged settings `includeGitInstructions`, defaulting true.
pub fn should_include_git_instructions(project_root: &std::path::Path) -> bool {
    const ENV_NAME: &str = "CLAUDE_CODE_DISABLE_GIT_INSTRUCTIONS";
    if crate::errors_util::is_env_truthy(ENV_NAME) {
        return false;
    }
    if crate::errors_util::is_env_definitely_falsy(ENV_NAME) {
        return true;
    }

    crate::permissions::load_permission_settings_value(project_root)
        .get("includeGitInstructions")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn env_truthy_disables_git_instructions() {
        let _guard = env_lock().lock().unwrap();
        std::env::set_var("CLAUDE_CODE_DISABLE_GIT_INSTRUCTIONS", "true");
        let tmp = tempfile::tempdir().unwrap();

        assert!(!should_include_git_instructions(tmp.path()));

        std::env::remove_var("CLAUDE_CODE_DISABLE_GIT_INSTRUCTIONS");
    }

    #[test]
    fn env_falsy_forces_git_instructions() {
        let _guard = env_lock().lock().unwrap();
        std::env::set_var("CLAUDE_CODE_DISABLE_GIT_INSTRUCTIONS", "false");
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".claude")).unwrap();
        std::fs::write(
            tmp.path().join(".claude/settings.json"),
            r#"{"includeGitInstructions": false}"#,
        )
        .unwrap();

        assert!(should_include_git_instructions(tmp.path()));

        std::env::remove_var("CLAUDE_CODE_DISABLE_GIT_INSTRUCTIONS");
    }

    #[test]
    fn settings_can_disable_git_instructions() {
        let _guard = env_lock().lock().unwrap();
        std::env::remove_var("CLAUDE_CODE_DISABLE_GIT_INSTRUCTIONS");
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".claude")).unwrap();
        std::fs::write(
            tmp.path().join(".claude/settings.json"),
            r#"{"includeGitInstructions": false}"#,
        )
        .unwrap();

        assert!(!should_include_git_instructions(tmp.path()));
    }
}
