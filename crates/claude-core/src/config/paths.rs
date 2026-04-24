use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Returns the path to the `~/.claude/` directory.
pub fn claude_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not determine home directory")?;
    Ok(home.join(".claude"))
}

/// Returns `~/.claude/settings.json`.
pub fn user_settings_path() -> Result<PathBuf> {
    Ok(claude_dir()?.join("settings.json"))
}

/// Returns `~/.claude/sessions/`.
pub fn sessions_dir() -> Result<PathBuf> {
    Ok(claude_dir()?.join("sessions"))
}

/// Walk up from `start` looking for common project-root markers.
/// Returns the directory containing the first marker found, or `start` itself
/// if no marker is found.
pub fn detect_project_root(start: &Path) -> PathBuf {
    const MARKERS: &[&str] = &[
        ".git",
        "Cargo.toml",
        "package.json",
        "go.mod",
        "pyproject.toml",
        "Makefile",
        ".hg",
        "pom.xml",
        "build.gradle",
    ];

    let mut current = start.to_path_buf();
    loop {
        for marker in MARKERS {
            if current.join(marker).exists() {
                return current;
            }
        }
        match current.parent() {
            Some(parent) => current = parent.to_path_buf(),
            None => return start.to_path_buf(),
        }
    }
}
