//! Sandbox executor: wraps bash commands in an OS-level sandbox.

use std::path::{Path, PathBuf};
use super::tracker::ViolationStore;
use super::types::*;

/// Wraps bash commands in an OS-level sandbox.
#[derive(Debug)]
pub struct SandboxExecutor {
    config: SandboxRuntimeConfig,
    active: bool,
    violation_store: ViolationStore,
    platform: Option<SandboxPlatform>,
}

impl SandboxExecutor {
    pub fn new(settings: &SandboxSettings, working_directory: PathBuf) -> Self {
        let platform = SandboxPlatform::current();
        let config = resolve_config(settings, &working_directory);
        let deps = Self::check_dependencies_for_platform(platform);
        let enabled = settings.enabled.unwrap_or(false);
        let platform_in_list = is_platform_in_enabled_list(settings, platform);
        let active = enabled && platform.is_some() && deps.is_available() && platform_in_list;
        Self { config, active, violation_store: ViolationStore::new(), platform }
    }

    pub fn is_active(&self) -> bool { self.active }
    pub fn violation_store(&self) -> &ViolationStore { &self.violation_store }

    /// Check whether a command should run inside the sandbox.
    pub fn should_sandbox_command(&self, command: &str, dangerously_disable: bool) -> bool {
        if !self.active { return false; }
        if dangerously_disable { return false; }
        if command.is_empty() { return false; }
        if self.is_command_excluded(command) { return false; }
        true
    }

    /// Wrap a command for sandboxed execution. Returns None if inactive.
    pub fn wrap_command(&self, command: &str) -> Option<String> {
        if !self.active { return None; }
        match self.platform? {
            SandboxPlatform::Macos => Some(self.wrap_macos(command)),
            SandboxPlatform::Linux | SandboxPlatform::Wsl => Some(self.wrap_linux(command)),
        }
    }

    /// Process sandboxed command results: parse violations, annotate stderr.
    pub fn process_result(&self, command: &str, stdout: String, stderr: String, exit_code: i32, interrupted: bool) -> SandboxResult {
        let violations = super::tracker::parse_violations_from_stderr(&stderr, command);
        for v in &violations { self.violation_store.record(v.clone()); }
        let annotated = if violations.is_empty() { stderr }
                        else { super::tracker::annotate_stderr_with_violations(command, &stderr) };
        SandboxResult { stdout, stderr: annotated, exit_code, interrupted, violations }
    }

    /// Cleanup after sandboxed command (remove bwrap mount stubs).
    pub fn cleanup_after_command(&self) {
        for name in &["HEAD", "objects", "refs", "hooks", "config"] {
            let path = self.config.working_directory.join(name);
            if let Ok(m) = std::fs::metadata(&path) {
                if m.len() == 0 && m.is_file() { let _ = std::fs::remove_file(&path); }
            }
        }
    }

    pub fn update_config(&mut self, settings: &SandboxSettings, working_directory: &Path) {
        self.config = resolve_config(settings, working_directory);
        let enabled = settings.enabled.unwrap_or(false);
        let deps = Self::check_dependencies_for_platform(self.platform);
        let platform_in_list = is_platform_in_enabled_list(settings, self.platform);
        self.active = enabled && self.platform.is_some() && deps.is_available() && platform_in_list;
    }

    pub fn reset(&mut self) { self.violation_store.clear(); }

    pub fn is_supported_platform() -> bool { SandboxPlatform::current().is_some() }

    pub fn check_dependencies() -> SandboxDependencyCheck {
        Self::check_dependencies_for_platform(SandboxPlatform::current())
    }

    fn check_dependencies_for_platform(platform: Option<SandboxPlatform>) -> SandboxDependencyCheck {
        let mut check = SandboxDependencyCheck::default();
        match platform {
            None => { check.errors.push("Unsupported platform".into()); }
            Some(SandboxPlatform::Macos) => {
                if !Path::new("/usr/bin/sandbox-exec").exists() {
                    check.errors.push("sandbox-exec not found".into());
                }
            }
            Some(SandboxPlatform::Linux) | Some(SandboxPlatform::Wsl) => {
                if which("bwrap").is_none() {
                    check.errors.push("bubblewrap (bwrap) not found".into());
                }
                if which("socat").is_none() {
                    check.warnings.push("socat not found".into());
                }
            }
        }
        check
    }

    pub fn is_auto_allow_bash_enabled(settings: &SandboxSettings) -> bool {
        settings.auto_allow_bash_if_sandboxed.unwrap_or(true)
    }

    pub fn get_unavailable_reason(settings: &SandboxSettings) -> Option<String> {
        if !settings.enabled.unwrap_or(false) { return None; }
        if SandboxPlatform::current().is_none() {
            return Some(format!("sandbox.enabled is set but {} is not supported", std::env::consts::OS));
        }
        if !is_platform_in_enabled_list(settings, SandboxPlatform::current()) {
            return Some("sandbox.enabled is set but platform not in enabledPlatforms".into());
        }
        let deps = Self::check_dependencies();
        if !deps.is_available() {
            return Some(format!("sandbox.enabled but deps missing: {}", deps.errors.join(", ")));
        }
        None
    }

    fn is_command_excluded(&self, command: &str) -> bool {
        let excluded = match &self.config.excluded_commands {
            Some(cmds) if !cmds.is_empty() => cmds,
            _ => return false,
        };
        let sub_commands = split_compound_for_exclusion(command);
        for sub_cmd in &sub_commands {
            let trimmed = sub_cmd.trim();
            for pattern in excluded {
                if matches_exclusion_pattern(pattern, trimmed) { return true; }
            }
        }
        false
    }

    fn wrap_macos(&self, command: &str) -> String {
        let profile = self.generate_macos_profile();
        let ep = profile.replace('\'', "'\\''");
        let ec = command.replace('\'', "'\\''");
        format!("sandbox-exec -p '{}' bash -c '{}'", ep, ec)
    }

    fn wrap_linux(&self, command: &str) -> String {
        let mut args: Vec<String> = Vec::new();
        args.extend(["--ro-bind", "/", "/", "--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp"].iter().map(|s| s.to_string()));
        for path in &self.config.filesystem.allow_write {
            if path.exists() { args.extend(["--bind".into(), path.display().to_string(), path.display().to_string()]); }
        }
        for path in &self.config.filesystem.deny_write {
            if path.exists() { args.extend(["--ro-bind".into(), path.display().to_string(), path.display().to_string()]); }
        }
        for path in &self.config.filesystem.deny_read {
            if path.exists() { args.extend(["--ro-bind".into(), "/dev/null".into(), path.display().to_string()]); }
        }
        if self.config.network.allowed_domains.is_empty() && self.config.network.http_proxy_port.is_none() && !self.config.network.allow_local_binding {
            args.push("--unshare-net".into());
        }
        args.extend(["--chdir".into(), self.config.working_directory.display().to_string(), "--die-with-parent".into()]);
        let ec = command.replace('\'', "'\\''");
        format!("bwrap {} -- bash -c '{}'", args.join(" "), ec)
    }

    fn generate_macos_profile(&self) -> String {
        let mut p = String::new();
        p.push_str("(version 1)\n(deny default)\n(allow process-exec)\n(allow process-fork)\n(allow file-read*)\n");
        for path in &self.config.filesystem.deny_read {
            p.push_str(&format!("(deny file-read* (subpath \"{}\"))\n", path.display()));
        }
        for path in &self.config.filesystem.allow_write {
            p.push_str(&format!("(allow file-write* (subpath \"{}\"))\n", path.display()));
        }
        for path in &self.config.filesystem.deny_write {
            p.push_str(&format!("(deny file-write* (subpath \"{}\"))\n", path.display()));
        }
        if !self.config.network.allowed_domains.is_empty() || self.config.network.allow_local_binding {
            p.push_str("(allow network*)\n");
        } else {
            p.push_str("(allow network* (local udp \"*:*\"))\n(allow network* (local tcp \"*:*\"))\n");
        }
        p.push_str("(allow sysctl-read)\n(allow mach-lookup)\n(allow signal)\n(allow iokit-open)\n");
        p
    }
}

fn resolve_config(settings: &SandboxSettings, wd: &Path) -> SandboxRuntimeConfig {
    let fs_cfg = settings.filesystem.as_ref();
    let net_cfg = settings.network.as_ref();
    let mut allow_write = vec![wd.to_path_buf()];
    allow_write.push(std::env::var("TMPDIR").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("/tmp")));
    let mut deny_write = Vec::new();
    let mut deny_read = Vec::new();
    let mut allow_read = Vec::new();
    if let Some(fs) = fs_cfg {
        for p in &fs.allow_write { allow_write.push(resolve_path(p, wd)); }
        for p in &fs.deny_write { deny_write.push(resolve_path(p, wd)); }
        for p in &fs.deny_read { deny_read.push(resolve_path(p, wd)); }
        for p in &fs.allow_read { allow_read.push(resolve_path(p, wd)); }
    }
    if let Some(home) = dirs::home_dir() {
        deny_write.push(home.join(".claude").join("settings.json"));
        deny_write.push(home.join(".claude").join("settings.local.json"));
    }
    deny_write.push(wd.join(".claude").join("settings.json"));
    deny_write.push(wd.join(".claude").join("settings.local.json"));
    deny_write.push(wd.join(".claude").join("skills"));
    SandboxRuntimeConfig {
        working_directory: wd.to_path_buf(),
        filesystem: ResolvedFilesystemConfig { allow_write, deny_write, deny_read, allow_read },
        network: ResolvedNetworkConfig {
            allowed_domains: net_cfg.map(|n| n.allowed_domains.clone()).unwrap_or_default(),
            denied_domains: Vec::new(),
            allow_unix_sockets: net_cfg.and_then(|n| n.allow_unix_sockets.clone()),
            allow_all_unix_sockets: net_cfg.and_then(|n| n.allow_all_unix_sockets).unwrap_or(false),
            allow_local_binding: net_cfg.and_then(|n| n.allow_local_binding).unwrap_or(false),
            http_proxy_port: net_cfg.and_then(|n| n.http_proxy_port),
            socks_proxy_port: net_cfg.and_then(|n| n.socks_proxy_port),
        },
        ignore_violations: settings.ignore_violations.clone().unwrap_or_default(),
        enable_weaker_nested_sandbox: settings.enable_weaker_nested_sandbox.unwrap_or(false),
        enable_weaker_network_isolation: settings.enable_weaker_network_isolation.unwrap_or(false),
        excluded_commands: settings.excluded_commands.clone(),
    }
}

fn resolve_path(pattern: &str, base: &Path) -> PathBuf {
    if pattern.starts_with("~/") {
        if let Some(home) = dirs::home_dir() { return home.join(&pattern[2..]); }
    }
    if pattern.starts_with("//") { return PathBuf::from(&pattern[1..]); }
    if pattern.starts_with('/') { return PathBuf::from(pattern); }
    if pattern.starts_with("./") { return base.join(&pattern[2..]); }
    base.join(pattern)
}

fn which(binary: &str) -> Option<PathBuf> {
    std::env::var("PATH").ok().and_then(|path_var| {
        path_var.split(':').find_map(|dir| {
            let c = PathBuf::from(dir).join(binary);
            if c.exists() { Some(c) } else { None }
        })
    })
}

fn is_platform_in_enabled_list(settings: &SandboxSettings, platform: Option<SandboxPlatform>) -> bool {
    match &settings.enabled_platforms {
        None => true,
        Some(platforms) if platforms.is_empty() => false,
        Some(platforms) => platform.map(|p| platforms.contains(&p)).unwrap_or(false),
    }
}

fn split_compound_for_exclusion(command: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_sq = false;
    let mut in_dq = false;
    let mut prev: Option<char> = None;
    let mut chars = command.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\'' && !in_dq && prev != Some('\\') { in_sq = !in_sq; current.push(ch); }
        else if ch == '"' && !in_sq && prev != Some('\\') { in_dq = !in_dq; current.push(ch); }
        else if !in_sq && !in_dq {
            match ch {
                '&' if chars.peek() == Some(&'&') => { chars.next(); let t = current.trim().to_string(); if !t.is_empty() { parts.push(t); } current.clear(); }
                '|' if chars.peek() == Some(&'|') => { chars.next(); let t = current.trim().to_string(); if !t.is_empty() { parts.push(t); } current.clear(); }
                ';' => { let t = current.trim().to_string(); if !t.is_empty() { parts.push(t); } current.clear(); }
                _ => { current.push(ch); }
            }
        } else { current.push(ch); }
        prev = Some(ch);
    }
    let t = current.trim().to_string();
    if !t.is_empty() { parts.push(t); }
    parts
}

fn matches_exclusion_pattern(pattern: &str, command: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix(":*") {
        return command == prefix || command.starts_with(&format!("{} ", prefix));
    }
    if pattern.contains('*') { return matches_wildcard(pattern, command); }
    command == pattern || command.starts_with(&format!("{} ", pattern))
}

fn matches_wildcard(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0, 0);
    let (mut p_star, mut t_star): (Option<usize>, Option<usize>) = (None, None);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == t[ti] || p[pi] == '?') { pi += 1; ti += 1; }
        else if pi < p.len() && p[pi] == '*' { p_star = Some(pi); t_star = Some(ti); pi += 1; }
        else if let Some(ps) = p_star { pi = ps + 1; let ts = t_star.unwrap() + 1; t_star = Some(ts); ti = ts; }
        else { return false; }
    }
    while pi < p.len() && p[pi] == '*' { pi += 1; }
    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_disabled_by_default() {
        let e = SandboxExecutor::new(&SandboxSettings::default(), PathBuf::from("/tmp"));
        assert!(!e.is_active());
        assert!(!e.should_sandbox_command("echo hi", false));
    }

    #[test]
    fn test_empty_not_sandboxed() {
        let mut e = SandboxExecutor::new(&SandboxSettings { enabled: Some(true), ..Default::default() }, PathBuf::from("/tmp"));
        e.active = true;
        assert!(!e.should_sandbox_command("", false));
    }

    #[test]
    fn test_dangerously_disable() {
        let mut e = SandboxExecutor::new(&SandboxSettings { enabled: Some(true), ..Default::default() }, PathBuf::from("/tmp"));
        e.active = true;
        assert!(!e.should_sandbox_command("echo hi", true));
        assert!(e.should_sandbox_command("echo hi", false));
    }

    #[test]
    fn test_excluded_commands() {
        let s = SandboxSettings { enabled: Some(true), excluded_commands: Some(vec!["npm test".into(), "cargo:*".into()]), ..Default::default() };
        let mut e = SandboxExecutor::new(&s, PathBuf::from("/tmp"));
        e.active = true;
        assert!(!e.should_sandbox_command("npm test", false));
        assert!(!e.should_sandbox_command("cargo build", false));
        assert!(e.should_sandbox_command("echo hi", false));
    }

    #[test]
    fn test_wrap_none_when_inactive() {
        let e = SandboxExecutor::new(&SandboxSettings::default(), PathBuf::from("/tmp"));
        assert!(e.wrap_command("echo hi").is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_wrap_macos() {
        let mut e = SandboxExecutor::new(&SandboxSettings { enabled: Some(true), ..Default::default() }, PathBuf::from("/tmp"));
        e.active = true;
        let w = e.wrap_command("echo hello").unwrap();
        assert!(w.contains("sandbox-exec"));
        assert!(w.contains("echo hello"));
    }

    #[test]
    fn test_process_result_no_violations() {
        let e = SandboxExecutor::new(&SandboxSettings::default(), PathBuf::from("/tmp"));
        let r = e.process_result("echo", "hi\n".into(), String::new(), 0, false);
        assert!(r.violations.is_empty());
    }

    #[test]
    fn test_process_result_with_violations() {
        let e = SandboxExecutor::new(&SandboxSettings::default(), PathBuf::from("/tmp"));
        let r = e.process_result("cmd", String::new(), "Sandbox: bash(1) deny(1) file-write-data /etc/passwd".into(), 1, false);
        assert_eq!(r.violations.len(), 1);
        assert!(r.stderr.contains("<sandbox_violations>"));
        assert_eq!(e.violation_store().count(), 1);
    }

    #[test]
    fn test_cleanup_removes_zero_byte_stubs() {
        let dir = tempfile::tempdir().unwrap();
        let head = dir.path().join("HEAD");
        std::fs::write(&head, "").unwrap();
        let e = SandboxExecutor::new(&SandboxSettings::default(), dir.path().to_path_buf());
        e.cleanup_after_command();
        assert!(!head.exists());
    }

    #[test]
    fn test_cleanup_keeps_nonempty() {
        let dir = tempfile::tempdir().unwrap();
        let head = dir.path().join("HEAD");
        std::fs::write(&head, "ref: refs/heads/main\n").unwrap();
        let e = SandboxExecutor::new(&SandboxSettings::default(), dir.path().to_path_buf());
        e.cleanup_after_command();
        assert!(head.exists());
    }

    #[test]
    fn test_resolve_path() {
        assert_eq!(resolve_path("/etc/hosts", Path::new("/tmp")), PathBuf::from("/etc/hosts"));
        assert_eq!(resolve_path("sub/f", Path::new("/base")), PathBuf::from("/base/sub/f"));
        assert_eq!(resolve_path("./sub/f", Path::new("/base")), PathBuf::from("/base/sub/f"));
        assert_eq!(resolve_path("//.aws/creds", Path::new("/base")), PathBuf::from("/.aws/creds"));
    }

    #[test]
    fn test_matches_exclusion() {
        assert!(matches_exclusion_pattern("npm test", "npm test"));
        assert!(!matches_exclusion_pattern("npm test", "npm run test"));
        assert!(matches_exclusion_pattern("cargo:*", "cargo build"));
        assert!(matches_exclusion_pattern("cargo:*", "cargo"));
        assert!(!matches_exclusion_pattern("cargo:*", "npm test"));
    }

    #[test]
    fn test_wildcard() {
        assert!(matches_wildcard("npm run test*", "npm run test:unit"));
        assert!(matches_wildcard("npm run test*", "npm run test"));
        assert!(!matches_wildcard("npm run test*", "npm run build"));
    }

    #[test]
    fn test_platform_enabled_list() {
        let s = SandboxSettings::default();
        assert!(is_platform_in_enabled_list(&s, Some(SandboxPlatform::Macos)));
        let s2 = SandboxSettings { enabled_platforms: Some(vec![]), ..Default::default() };
        assert!(!is_platform_in_enabled_list(&s2, Some(SandboxPlatform::Macos)));
        let s3 = SandboxSettings { enabled_platforms: Some(vec![SandboxPlatform::Macos]), ..Default::default() };
        assert!(is_platform_in_enabled_list(&s3, Some(SandboxPlatform::Macos)));
        assert!(!is_platform_in_enabled_list(&s3, Some(SandboxPlatform::Linux)));
    }

    #[test]
    fn test_split_compound() {
        assert_eq!(split_compound_for_exclusion("a && b"), vec!["a", "b"]);
        assert_eq!(split_compound_for_exclusion("a; b || c"), vec!["a", "b", "c"]);
        assert_eq!(split_compound_for_exclusion("echo 'a && b' && c"), vec!["echo 'a && b'", "c"]);
    }

    #[test]
    fn test_which_bash() {
        assert!(which("bash").is_some());
    }

    #[test]
    fn test_unavailable_reason_disabled() {
        assert!(SandboxExecutor::get_unavailable_reason(&SandboxSettings::default()).is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_deps_macos() {
        let c = SandboxExecutor::check_dependencies_for_platform(Some(SandboxPlatform::Macos));
        assert!(c.is_available());
    }
}
