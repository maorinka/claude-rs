use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;
use tokio::process::Command;

const SINGLE_FILE_DIFF_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_DIFF_SIZE_BYTES: u64 = 1_000_000;

pub(crate) fn should_include_tool_git_diff() -> bool {
    claude_core::errors_util::is_env_truthy("CLAUDE_CODE_REMOTE")
        && claude_core::growthbook::get_feature_value_cached_may_be_stale_bool(
            "tengu_quartz_lantern",
            false,
        )
}

pub(crate) async fn fetch_single_file_git_diff(absolute_file_path: &Path) -> Option<Value> {
    let parent = absolute_file_path.parent()?;
    let git_root = claude_core::find_git_root::find_git_root(parent)?;
    let canonical_file = absolute_file_path
        .canonicalize()
        .unwrap_or_else(|_| absolute_file_path.to_path_buf());
    let git_path = git_relative_path(&git_root, &canonical_file)?;
    let repository = cached_repository();

    if git_ls_files_tracked(&git_root, &git_path).await {
        let diff_ref = get_diff_ref(&git_root).await;
        let stdout = git_output(
            &git_root,
            &["--no-optional-locks", "diff", &diff_ref, "--", &git_path],
        )
        .await?;
        if stdout.is_empty() {
            return None;
        }
        return Some(parse_raw_diff_to_tool_use_diff(
            &git_path, &stdout, "modified", repository,
        ));
    }

    generate_synthetic_diff(&git_path, absolute_file_path, repository).await
}

fn git_relative_path(git_root: &Path, absolute_file_path: &Path) -> Option<String> {
    let rel = absolute_file_path.strip_prefix(git_root).ok()?;
    Some(
        rel.components()
            .map(|component| component.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/"),
    )
}

async fn git_ls_files_tracked(git_root: &Path, git_path: &str) -> bool {
    git_status(
        git_root,
        &[
            "--no-optional-locks",
            "ls-files",
            "--error-unmatch",
            git_path,
        ],
    )
    .await
    .unwrap_or(false)
}

async fn get_diff_ref(git_root: &Path) -> String {
    let base_branch = std::env::var("CLAUDE_CODE_BASE_REF")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| default_branch(git_root));

    if let Some(stdout) = git_output(
        git_root,
        &["--no-optional-locks", "merge-base", "HEAD", &base_branch],
    )
    .await
    {
        let trimmed = stdout.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    "HEAD".to_string()
}

fn default_branch(git_root: &Path) -> String {
    if let Ok(output) = std::process::Command::new("git")
        .args(["symbolic-ref", "refs/remotes/origin/HEAD", "--short"])
        .current_dir(git_root)
        .output()
    {
        if output.status.success() {
            let branch = String::from_utf8_lossy(&output.stdout);
            let branch = branch.trim();
            if !branch.is_empty() {
                return branch
                    .rsplit_once('/')
                    .map(|(_, name)| name)
                    .unwrap_or(branch)
                    .to_string();
            }
        }
    }

    for candidate in ["main", "master"] {
        if std::process::Command::new("git")
            .args([
                "rev-parse",
                "--verify",
                &format!("refs/remotes/origin/{candidate}"),
            ])
            .current_dir(git_root)
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
        {
            return candidate.to_string();
        }
    }

    "main".to_string()
}

fn parse_raw_diff_to_tool_use_diff(
    filename: &str,
    raw_diff: &str,
    status: &str,
    repository: Option<String>,
) -> Value {
    let mut patch_lines = Vec::new();
    let mut in_hunks = false;
    let mut additions = 0usize;
    let mut deletions = 0usize;

    for line in raw_diff.split('\n') {
        if line.starts_with("@@") {
            in_hunks = true;
        }
        if in_hunks {
            patch_lines.push(line);
            if line.starts_with('+') && !line.starts_with("+++") {
                additions += 1;
            } else if line.starts_with('-') && !line.starts_with("---") {
                deletions += 1;
            }
        }
    }

    json!({
        "filename": filename,
        "status": status,
        "additions": additions,
        "deletions": deletions,
        "changes": additions + deletions,
        "patch": patch_lines.join("\n"),
        "repository": repository,
    })
}

async fn generate_synthetic_diff(
    git_path: &str,
    absolute_file_path: &Path,
    repository: Option<String>,
) -> Option<Value> {
    let metadata = tokio::fs::metadata(absolute_file_path).await.ok()?;
    if metadata.len() > MAX_DIFF_SIZE_BYTES {
        return None;
    }

    let content = tokio::fs::read_to_string(absolute_file_path).await.ok()?;
    let mut lines: Vec<&str> = content.split('\n').collect();
    if lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    let line_count = lines.len();
    let added_lines = lines
        .iter()
        .map(|line| format!("+{}", line))
        .collect::<Vec<_>>()
        .join("\n");
    let patch = format!("@@ -0,0 +1,{} @@\n{}", line_count, added_lines);

    Some(json!({
        "filename": git_path,
        "status": "added",
        "additions": line_count,
        "deletions": 0,
        "changes": line_count,
        "patch": patch,
        "repository": repository,
    }))
}

async fn git_status(git_root: &Path, args: &[&str]) -> Option<bool> {
    let output = run_git(git_root, args).await?;
    Some(output.status.success())
}

async fn git_output(git_root: &Path, args: &[&str]) -> Option<String> {
    let output = run_git(git_root, args).await?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).to_string())
}

async fn run_git(git_root: &Path, args: &[&str]) -> Option<std::process::Output> {
    tokio::time::timeout(
        SINGLE_FILE_DIFF_TIMEOUT,
        Command::new("git")
            .args(args)
            .current_dir(git_root)
            .output(),
    )
    .await
    .ok()?
    .ok()
}

fn cached_repository() -> Option<String> {
    std::env::var("CLAUDE_CODE_GITHUB_REPOSITORY")
        .ok()
        .or_else(|| std::env::var("GITHUB_REPOSITORY").ok())
        .filter(|value| !value.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn git(dir: &Path, args: &[&str]) -> bool {
        std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    fn init_repo() -> Option<tempfile::TempDir> {
        if std::process::Command::new("git")
            .arg("--version")
            .output()
            .ok()
            .filter(|output| output.status.success())
            .is_none()
        {
            return None;
        }
        let dir = tempfile::tempdir().ok()?;
        if !git(dir.path(), &["init", "-b", "main"]) {
            return None;
        }
        let _ = git(dir.path(), &["config", "user.email", "test@example.com"]);
        let _ = git(dir.path(), &["config", "user.name", "Test User"]);
        Some(dir)
    }

    #[test]
    fn tool_git_diff_gate_matches_ts_shape() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("CLAUDE_CODE_REMOTE");
        std::env::remove_var("CLAUDE_CODE_GROWTHBOOK_TENGU_QUARTZ_LANTERN");
        assert!(!should_include_tool_git_diff());
        std::env::set_var("CLAUDE_CODE_REMOTE", "1");
        assert!(!should_include_tool_git_diff());
        std::env::set_var("CLAUDE_CODE_GROWTHBOOK_TENGU_QUARTZ_LANTERN", "1");
        assert!(should_include_tool_git_diff());
        std::env::remove_var("CLAUDE_CODE_REMOTE");
        std::env::remove_var("CLAUDE_CODE_GROWTHBOOK_TENGU_QUARTZ_LANTERN");
    }

    #[tokio::test]
    async fn fetches_tracked_file_diff() {
        let Some(dir) = init_repo() else {
            return;
        };
        let path = dir.path().join("a.txt");
        std::fs::write(&path, "old\n").unwrap();
        assert!(git(dir.path(), &["add", "a.txt"]));
        assert!(git(dir.path(), &["commit", "-m", "init"]));
        std::fs::write(&path, "old\nnew\n").unwrap();

        let diff = fetch_single_file_git_diff(&path).await.unwrap();
        assert_eq!(diff["filename"], "a.txt");
        assert_eq!(diff["status"], "modified");
        assert_eq!(diff["additions"], 1);
        assert!(diff["patch"].as_str().unwrap().contains("+new"));
    }

    #[tokio::test]
    async fn fetches_untracked_synthetic_diff() {
        let Some(dir) = init_repo() else {
            return;
        };
        let path = dir.path().join("new.txt");
        std::fs::write(&path, "one\ntwo\n").unwrap();

        let diff = fetch_single_file_git_diff(&path).await.unwrap();
        assert_eq!(diff["filename"], "new.txt");
        assert_eq!(diff["status"], "added");
        assert_eq!(diff["additions"], 2);
        assert_eq!(diff["patch"], "@@ -0,0 +1,2 @@\n+one\n+two");
    }
}
