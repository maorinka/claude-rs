/// Build the environment info section for the system prompt.
///
/// Matches TS `computeEnvInfo` in `src/constants/prompts.ts:640-648`:
/// ```text
/// Here is useful information about the environment you are running in:
/// <env>
/// Working directory: ...
/// Is directory a git repo: Yes|No
/// Platform: ...
/// Shell: ...
/// OS Version: ...
/// </env>
/// ```
pub fn build_environment_context() -> String {
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "unknown".into());

    let is_git = std::process::Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "unknown".into());

    let os_version = std::process::Command::new("uname")
        .args(["-sr"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| format!("{} {}", std::env::consts::OS, std::env::consts::ARCH));

    format!(
        "Here is useful information about the environment you are running in:\n\
         <env>\n\
         Working directory: {cwd}\n\
         Is directory a git repo: {is_git}\n\
         Platform: {platform}\n\
         Shell: {shell}\n\
         OS Version: {os_version}\n\
         </env>",
        cwd = cwd,
        is_git = if is_git { "Yes" } else { "No" },
        platform = std::env::consts::OS,
        shell = shell,
        os_version = os_version,
    )
}
