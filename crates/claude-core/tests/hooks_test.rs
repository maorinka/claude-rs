use claude_core::hooks::matching::{evaluate_if_condition, glob_match};
use claude_core::hooks::ssrf::is_blocked_address;
use claude_core::hooks::types::HookEvent;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

// ── evaluate_if_condition tests ──────────────────────────────────────────

#[test]
fn if_condition_matches_tool_name_only() {
    let input = serde_json::json!({
        "tool_name": "Bash",
        "tool_input": {"command": "ls -la"}
    });
    assert!(evaluate_if_condition("Bash", &HookEvent::PreToolUse, &input));
}

#[test]
fn if_condition_rejects_wrong_tool_name() {
    let input = serde_json::json!({
        "tool_name": "Read",
        "tool_input": {"file_path": "/tmp/foo"}
    });
    assert!(!evaluate_if_condition("Bash", &HookEvent::PreToolUse, &input));
}

#[test]
fn if_condition_with_prefix_pattern() {
    let input = serde_json::json!({
        "tool_name": "Bash",
        "tool_input": {"command": "git push origin main"}
    });
    assert!(evaluate_if_condition("Bash(git )", &HookEvent::PreToolUse, &input));
}

#[test]
fn if_condition_with_glob_pattern() {
    let input = serde_json::json!({
        "tool_name": "Bash",
        "tool_input": {"command": "rm -rf /tmp/test"}
    });
    assert!(evaluate_if_condition("Bash(rm *)", &HookEvent::PreToolUse, &input));
}

#[test]
fn if_condition_non_tool_event_returns_false() {
    let input = serde_json::json!({"tool_name": "Bash"});
    assert!(!evaluate_if_condition("Bash", &HookEvent::Stop, &input));
}

#[test]
fn if_condition_no_tool_name_in_input_returns_false() {
    let input = serde_json::json!({"other": "data"});
    assert!(!evaluate_if_condition("Bash", &HookEvent::PreToolUse, &input));
}

// ── glob_match tests ────────────────────────────────────────────────────

#[test]
fn glob_match_wildcard_matches_everything() {
    assert!(glob_match("*", "anything"));
}

#[test]
fn glob_match_prefix() {
    assert!(glob_match("git ", "git status"));
    assert!(!glob_match("git ", "npm install"));
}

#[test]
fn glob_match_trailing_star() {
    assert!(glob_match("npm *", "npm install"));
    assert!(!glob_match("npm *", "git push"));
}

#[test]
fn glob_match_exact() {
    assert!(glob_match("ls", "ls"));
}

// ── SSRF guard tests ────────────────────────────────────────────────────

#[test]
fn ssrf_blocks_private_ranges() {
    assert!(is_blocked_address(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
    assert!(is_blocked_address(IpAddr::V4(Ipv4Addr::new(172, 16, 0, 1))));
    assert!(is_blocked_address(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))));
    assert!(is_blocked_address(IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254))));
    assert!(is_blocked_address(IpAddr::V4(Ipv4Addr::new(100, 100, 100, 200))));
}

#[test]
fn ssrf_allows_loopback() {
    assert!(!is_blocked_address(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))));
    assert!(!is_blocked_address(IpAddr::V6(Ipv6Addr::LOCALHOST)));
}

#[test]
fn ssrf_allows_public_ips() {
    assert!(!is_blocked_address(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
    assert!(!is_blocked_address(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))));
}

#[test]
fn ssrf_blocks_ipv6_private() {
    assert!(is_blocked_address(IpAddr::V6(Ipv6Addr::new(0xfc00, 0, 0, 0, 0, 0, 0, 1))));
    assert!(is_blocked_address(IpAddr::V6(Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1))));
    assert!(is_blocked_address(IpAddr::V6(Ipv6Addr::UNSPECIFIED)));
}
