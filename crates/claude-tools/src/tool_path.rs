use std::path::{Path, PathBuf};

pub(crate) fn expand_tool_path(path: &str, cwd: &Path) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return cwd.to_string_lossy().to_string();
    }

    let expanded = claude_core::permissions::expand_path(trimmed);
    let absolute = if expanded.is_absolute() {
        expanded
    } else {
        cwd.join(expanded)
    };

    normalize_path(&absolute).to_string_lossy().to_string()
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                result.pop();
            }
            other => result.push(other),
        }
    }
    if result.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        result
    }
}
