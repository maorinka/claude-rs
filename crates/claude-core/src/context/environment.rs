pub fn build_environment_context() -> String {
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "unknown".into());

    format!(
        "# Environment\n\
         - Platform: {}\n\
         - Architecture: {}\n\
         - Shell: {}\n\
         - Working directory: {}\n",
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::env::var("SHELL").unwrap_or_else(|_| "unknown".into()),
        cwd,
    )
}
