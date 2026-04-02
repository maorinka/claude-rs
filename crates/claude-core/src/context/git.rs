use anyhow::Result;
use std::path::Path;
use tokio::process::Command;

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

    let mut context = String::new();

    // Branch
    if let Ok(branch) = git_output(project_root, &["branch", "--show-current"]).await {
        let branch = branch.trim();
        if !branch.is_empty() {
            context.push_str(&format!("Current branch: {}\n", branch));
        }
    }

    // Recent commits
    if let Ok(log) = git_output(project_root, &["log", "--oneline", "-5", "--no-decorate"]).await {
        if !log.trim().is_empty() {
            context.push_str(&format!("Recent commits:\n{}\n", log.trim()));
        }
    }

    // Status
    if let Ok(status) = git_output(project_root, &["status", "--short"]).await {
        if !status.trim().is_empty() {
            context.push_str(&format!("Working tree status:\n{}\n", status.trim()));
        }
    }

    if context.is_empty() {
        Ok(None)
    } else {
        Ok(Some(context))
    }
}

async fn git_output(dir: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .await?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}
