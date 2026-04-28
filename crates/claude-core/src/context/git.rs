use anyhow::Result;
use std::path::Path;
use tokio::process::Command;

const MAX_STATUS_CHARS: usize = 2000;

/// Build the git status context prepended to conversations.
///
/// Matches TS `src/context.ts:96-103`:
/// ```text
/// This is the git status at the start of the conversation. ...
/// Current branch: <branch>
/// Main branch (you will usually use this for PRs): <main>
/// Status: <status>
/// Recent commits: <log>
/// ```
pub async fn get_git_context(project_root: &Path) -> Result<Option<String>> {
    // Check if in git repo
    let check = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(project_root)
        .output()
        .await?;

    if !check.status.success() {
        return Ok(None);
    }

    let mut parts: Vec<String> = Vec::new();

    parts.push(
        "This is the git status at the start of the conversation. Note that this status \
         is a snapshot in time, and will not update during the conversation."
            .to_string(),
    );

    // Current branch
    if let Ok(branch) = git_output(project_root, &["branch", "--show-current"]).await {
        let branch = branch.trim();
        if !branch.is_empty() {
            parts.push(format!("Current branch: {}", branch));
        }
    }

    // Main branch (prefer origin/HEAD, then local main/master)
    let main_branch = detect_main_branch(project_root).await;
    parts.push(format!(
        "Main branch (you will usually use this for PRs): {}",
        main_branch
    ));

    // Git user
    if let Ok(user_name) = git_output(project_root, &["config", "user.name"]).await {
        let user_name = user_name.trim();
        if !user_name.is_empty() {
            parts.push(format!("Git user: {}", user_name));
        }
    }

    // Status (truncated to avoid bloating context)
    if let Ok(status) =
        git_output(project_root, &["--no-optional-locks", "status", "--short"]).await
    {
        let status = status.trim();
        let truncated = if status.chars().count() > MAX_STATUS_CHARS {
            let truncated_str: String = status.chars().take(MAX_STATUS_CHARS).collect();
            format!(
                "{}\n... (truncated because it exceeds 2k characters. If you need more information, run \"git status\" using BashTool)",
                truncated_str
            )
        } else if status.is_empty() {
            "(clean)".to_string()
        } else {
            status.to_string()
        };
        parts.push(format!("Status:\n{}", truncated));
    }

    // Recent commits
    if let Ok(log) = git_output(
        project_root,
        &["--no-optional-locks", "log", "--oneline", "-n", "5"],
    )
    .await
    {
        if !log.trim().is_empty() {
            parts.push(format!("Recent commits:\n{}", log.trim()));
        }
    }

    Ok(Some(parts.join("\n\n")))
}

/// Detect the main branch name.
async fn detect_main_branch(project_root: &Path) -> String {
    if let Ok(output) = git_output(
        project_root,
        &["symbolic-ref", "refs/remotes/origin/HEAD", "--short"],
    )
    .await
    {
        let branch = output.trim();
        if !branch.is_empty() {
            return branch
                .rsplit_once('/')
                .map(|(_, name)| name)
                .unwrap_or(branch)
                .to_string();
        }
    }

    // Try 'main' first
    if let Ok(output) =
        git_output(project_root, &["rev-parse", "--verify", "refs/heads/main"]).await
    {
        if !output.trim().is_empty() {
            return "main".to_string();
        }
    }
    // Fallback to 'master'
    if let Ok(output) = git_output(
        project_root,
        &["rev-parse", "--verify", "refs/heads/master"],
    )
    .await
    {
        if !output.trim().is_empty() {
            return "master".to_string();
        }
    }
    // Default
    "main".to_string()
}

async fn git_output(dir: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .await?;
    if !output.status.success() {
        anyhow::bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}
