//! Sandbox violation tracking and stderr annotation.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use super::types::{SandboxViolation, ViolationKind};

const VIOLATION_TAG_OPEN: &str = "<sandbox_violations>";
const VIOLATION_TAG_CLOSE: &str = "</sandbox_violations>";

/// Thread-safe store for sandbox violations.
#[derive(Debug, Clone)]
pub struct ViolationStore {
    inner: Arc<Mutex<ViolationStoreInner>>,
}

#[derive(Debug, Default)]
struct ViolationStoreInner {
    by_command: HashMap<String, Vec<SandboxViolation>>,
    all: Vec<SandboxViolation>,
}

impl Default for ViolationStore {
    fn default() -> Self { Self::new() }
}

impl ViolationStore {
    pub fn new() -> Self {
        Self { inner: Arc::new(Mutex::new(ViolationStoreInner::default())) }
    }

    pub fn record(&self, violation: SandboxViolation) {
        let mut inner = self.inner.lock().unwrap();
        inner.by_command.entry(violation.command.clone()).or_default().push(violation.clone());
        inner.all.push(violation);
    }

    pub fn get_for_command(&self, command: &str) -> Vec<SandboxViolation> {
        self.inner.lock().unwrap().by_command.get(command).cloned().unwrap_or_default()
    }

    pub fn get_all(&self) -> Vec<SandboxViolation> {
        self.inner.lock().unwrap().all.clone()
    }

    pub fn count(&self) -> usize {
        self.inner.lock().unwrap().all.len()
    }

    pub fn clear(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.by_command.clear();
        inner.all.clear();
    }
}

/// Parse sandbox violations from stderr output.
pub fn parse_violations_from_stderr(stderr: &str, command: &str) -> Vec<SandboxViolation> {
    let mut violations = Vec::new();
    for line in stderr.lines() {
        let trimmed = line.trim();
        if let Some(v) = parse_macos_sandbox_deny(trimmed, command) {
            violations.push(v);
            continue;
        }
        if let Some(v) = parse_linux_bwrap_deny(trimmed, command) {
            violations.push(v);
            continue;
        }
        if (trimmed.contains("Permission denied") || trimmed.contains("Operation not permitted"))
            && !trimmed.contains("sudo")
        {
            if let Some(path) = extract_path_from_error(trimmed) {
                violations.push(SandboxViolation {
                    kind: ViolationKind::FsWrite, description: trimmed.into(),
                    target: path, command: command.into(), timestamp: None,
                });
            }
        }
        if trimmed.contains("Connection refused") || trimmed.contains("Network is unreachable")
            || (trimmed.contains("Could not resolve host") && !trimmed.contains("DNS"))
        {
            if let Some(host) = extract_host_from_error(trimmed) {
                violations.push(SandboxViolation {
                    kind: ViolationKind::Network, description: trimmed.into(),
                    target: host, command: command.into(), timestamp: None,
                });
            }
        }
    }
    violations
}

fn parse_macos_sandbox_deny(line: &str, command: &str) -> Option<SandboxViolation> {
    let rest = line.strip_prefix("Sandbox:")?.trim();
    if !rest.contains("deny(") { return None; }
    let deny_end = rest.find("deny(")?;
    let after_deny = &rest[deny_end..];
    let close_paren = after_deny.find(')')?;
    let op_and_path = after_deny[close_paren + 1..].trim();
    let (operation, path) = match op_and_path.split_once(' ') {
        Some((op, p)) => (op.trim(), p.trim().to_string()),
        None => (op_and_path, String::new()),
    };
    let kind = match operation {
        op if op.starts_with("file-write") => ViolationKind::FsWrite,
        op if op.starts_with("file-read") => ViolationKind::FsRead,
        op if op.starts_with("network") => ViolationKind::Network,
        op if op.starts_with("process-exec") => ViolationKind::ProcessExec,
        _ => ViolationKind::Other,
    };
    Some(SandboxViolation { kind, description: line.into(), target: path, command: command.into(), timestamp: None })
}

fn parse_linux_bwrap_deny(line: &str, command: &str) -> Option<SandboxViolation> {
    let rest = line.strip_prefix("bwrap:")?.trim();
    if rest.starts_with("Can't") {
        let path = rest.split_whitespace().nth(3)?.to_string();
        let kind = if rest.contains("writing") { ViolationKind::FsWrite }
                   else if rest.contains("reading") { ViolationKind::FsRead }
                   else { ViolationKind::Other };
        return Some(SandboxViolation { kind, description: line.into(), target: path, command: command.into(), timestamp: None });
    }
    None
}

fn extract_path_from_error(line: &str) -> Option<String> {
    if let Some(start) = line.find('\'') {
        if let Some(end) = line[start + 1..].find('\'') {
            let path = &line[start + 1..start + 1 + end];
            if path.starts_with('/') { return Some(path.to_string()); }
        }
    }
    for part in line.split(": ") {
        let trimmed = part.trim();
        if trimmed.starts_with('/') && !trimmed.contains(' ') {
            return Some(trimmed.to_string());
        }
    }
    None
}

fn extract_host_from_error(line: &str) -> Option<String> {
    if let Some(idx) = line.find("resolve host:") {
        let rest = line[idx + "resolve host:".len()..].trim();
        return Some(rest.split_whitespace().next()?.trim_end_matches('.').to_string());
    }
    if let Some(idx) = line.find("connect to ") {
        let rest = &line[idx + "connect to ".len()..];
        return Some(rest.split_whitespace().next()?.to_string());
    }
    None
}

/// Annotate stderr with sandbox violation XML tags.
pub fn annotate_stderr_with_violations(command: &str, stderr: &str) -> String {
    let violations = parse_violations_from_stderr(stderr, command);
    if violations.is_empty() { return stderr.to_string(); }
    let mut annotation = String::new();
    annotation.push_str(VIOLATION_TAG_OPEN);
    annotation.push('\n');
    for v in &violations {
        annotation.push_str(&format!("  [{:?}] {} (target: {})\n", v.kind, v.description, v.target));
    }
    annotation.push_str(VIOLATION_TAG_CLOSE);
    format!("{}\n{}", stderr, annotation)
}

/// Remove `<sandbox_violations>` tags from text.
pub fn remove_sandbox_violation_tags(text: &str) -> String {
    let mut result = text.to_string();
    while let Some(start) = result.find(VIOLATION_TAG_OPEN) {
        if let Some(end) = result[start..].find(VIOLATION_TAG_CLOSE) {
            let tag_end = start + end + VIOLATION_TAG_CLOSE.len();
            result = format!("{}{}", &result[..start], result[tag_end..].trim_start());
        } else { break; }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_store_record_and_retrieve() {
        let store = ViolationStore::new();
        store.record(SandboxViolation { kind: ViolationKind::FsWrite, description: "denied".into(), target: "/etc/passwd".into(), command: "cmd".into(), timestamp: None });
        assert_eq!(store.count(), 1);
        assert_eq!(store.get_for_command("cmd").len(), 1);
        assert!(store.get_for_command("other").is_empty());
    }

    #[test]
    fn test_store_clear() {
        let store = ViolationStore::new();
        store.record(SandboxViolation { kind: ViolationKind::Network, description: "blocked".into(), target: "evil.com".into(), command: "curl".into(), timestamp: None });
        store.clear();
        assert_eq!(store.count(), 0);
    }

    #[test]
    fn test_store_clone_shared() {
        let store = ViolationStore::new();
        let store2 = store.clone();
        store.record(SandboxViolation { kind: ViolationKind::FsWrite, description: "t".into(), target: "/t".into(), command: "t".into(), timestamp: None });
        assert_eq!(store2.count(), 1);
    }

    #[test]
    fn test_parse_macos_file_write() {
        let v = parse_violations_from_stderr("Sandbox: bash(12345) deny(1) file-write-data /etc/passwd", "cmd");
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].kind, ViolationKind::FsWrite);
        assert_eq!(v[0].target, "/etc/passwd");
    }

    #[test]
    fn test_parse_macos_network() {
        let v = parse_violations_from_stderr("Sandbox: curl(111) deny(1) network-outbound /private/var/run/mDNSResponder", "cmd");
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].kind, ViolationKind::Network);
    }

    #[test]
    fn test_parse_bwrap_write() {
        let v = parse_violations_from_stderr("bwrap: Can't open file /etc/passwd for writing: Operation not permitted", "cmd");
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].kind, ViolationKind::FsWrite);
    }

    #[test]
    fn test_parse_permission_denied() {
        let v = parse_violations_from_stderr("bash: /etc/shadow: Permission denied", "cmd");
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].target, "/etc/shadow");
    }

    #[test]
    fn test_parse_network_resolve() {
        let v = parse_violations_from_stderr("curl: (6) Could not resolve host: evil.com", "cmd");
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].kind, ViolationKind::Network);
        assert_eq!(v[0].target, "evil.com");
    }

    #[test]
    fn test_parse_no_violations() {
        assert!(parse_violations_from_stderr("warning: unused variable", "cmd").is_empty());
    }

    #[test]
    fn test_annotate_no_violations() {
        assert_eq!(annotate_stderr_with_violations("cmd", "ok"), "ok");
    }

    #[test]
    fn test_annotate_with_violations() {
        let r = annotate_stderr_with_violations("cmd", "Sandbox: bash(1) deny(1) file-write-data /etc/passwd");
        assert!(r.contains("<sandbox_violations>"));
        assert!(r.contains("/etc/passwd"));
    }

    #[test]
    fn test_remove_tags() {
        let t = "a\n<sandbox_violations>\ninfo\n</sandbox_violations>\nb";
        let c = remove_sandbox_violation_tags(t);
        assert!(!c.contains("<sandbox_violations>"));
        assert!(c.contains('a'));
        assert!(c.contains('b'));
    }

    #[test]
    fn test_remove_tags_none() {
        assert_eq!(remove_sandbox_violation_tags("normal"), "normal");
    }
}
