//! SSRF guard for HTTP hooks.
//!
//! Blocks private, link-local, and cloud-metadata address ranges to prevent
//! project-configured HTTP hooks from reaching internal infrastructure.
//! Loopback (127.0.0.0/8, ::1) is intentionally ALLOWED for local dev hooks.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Returns `true` if `addr` is in a range that HTTP hooks must not reach.
///
/// Blocked IPv4:
///   0.0.0.0/8        "this" network
///   10.0.0.0/8       private
///   100.64.0.0/10    shared / CGNAT (includes Alibaba metadata 100.100.100.200)
///   169.254.0.0/16   link-local / cloud metadata (AWS, GCP, Azure)
///   172.16.0.0/12    private
///   192.168.0.0/16   private
///
/// Blocked IPv6:
///   :: (unspecified)
///   fc00::/7         unique local
///   fe80::/10        link-local
///   ::ffff:<v4>      IPv4-mapped — delegated to v4 check
///
/// Allowed (returns false):
///   127.0.0.0/8, ::1  loopback
pub fn is_blocked_address(addr: IpAddr) -> bool {
    match addr {
        IpAddr::V4(v4) => is_blocked_v4(v4),
        IpAddr::V6(v6) => is_blocked_v6(v6),
    }
}

fn is_blocked_v4(addr: Ipv4Addr) -> bool {
    let o = addr.octets();
    let [a, b, ..] = o;

    // Loopback explicitly allowed.
    if a == 127 {
        return false;
    }

    // 0.0.0.0/8
    if a == 0 {
        return true;
    }
    // 10.0.0.0/8
    if a == 10 {
        return true;
    }
    // 169.254.0.0/16 — link-local, cloud metadata
    if a == 169 && b == 254 {
        return true;
    }
    // 172.16.0.0/12
    if a == 172 && (16..=31).contains(&b) {
        return true;
    }
    // 100.64.0.0/10 — CGNAT / shared address space (RFC 6598)
    if a == 100 && (64..=127).contains(&b) {
        return true;
    }
    // 192.168.0.0/16
    if a == 192 && b == 168 {
        return true;
    }

    false
}

fn is_blocked_v6(addr: Ipv6Addr) -> bool {
    // ::1 loopback explicitly allowed.
    if addr == Ipv6Addr::LOCALHOST {
        return false;
    }
    // :: unspecified
    if addr == Ipv6Addr::UNSPECIFIED {
        return true;
    }
    // IPv4-mapped (::ffff:a.b.c.d) — extract embedded v4 and delegate.
    if let Some(v4) = addr.to_ipv4_mapped() {
        return is_blocked_v4(v4);
    }
    let segs = addr.segments();
    let first = segs[0];
    // fc00::/7 — unique local (fc00 through fdff)
    if first & 0xfe00 == 0xfc00 {
        return true;
    }
    // fe80::/10 — link-local
    if first & 0xffc0 == 0xfe80 {
        return true;
    }

    false
}

/// Resolve `hostname` and return an error string if any resolved address is
/// in a blocked range. IP literals are validated directly.
///
/// Returns `Ok(())` if the host is safe to connect to, or `Err(msg)` with a
/// human-readable message if it is blocked.
pub async fn ssrf_check(hostname: &str, port: u16) -> Result<(), String> {
    // Try to parse as an IP literal first (avoids a DNS round-trip).
    if let Ok(ip) = hostname.parse::<IpAddr>() {
        if is_blocked_address(ip) {
            return Err(format!(
                "HTTP hook blocked: {} is a private/link-local address. \
                 Loopback (127.0.0.1, ::1) is allowed for local dev.",
                ip
            ));
        }
        return Ok(());
    }

    // DNS resolution via tokio's async resolver.
    let addr_str = format!("{}:{}", hostname, port);
    let addrs = tokio::net::lookup_host(&addr_str)
        .await
        .map_err(|e| format!("HTTP hook DNS resolution failed for {}: {}", hostname, e))?;

    for socket_addr in addrs {
        let ip = socket_addr.ip();
        if is_blocked_address(ip) {
            return Err(format!(
                "HTTP hook blocked: {} resolves to {} (private/link-local address). \
                 Loopback (127.0.0.1, ::1) is allowed for local dev.",
                hostname, ip
            ));
        }
    }

    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    #[test]
    fn test_loopback_allowed() {
        assert!(!is_blocked_address(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))));
        assert!(!is_blocked_address(IpAddr::V4(Ipv4Addr::new(127, 255, 255, 255))));
        assert!(!is_blocked_address(IpAddr::V6(Ipv6Addr::LOCALHOST)));
    }

    #[test]
    fn test_private_ranges_blocked() {
        // 0/8
        assert!(is_blocked_address(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0))));
        // 10/8
        assert!(is_blocked_address(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
        // 172.16/12
        assert!(is_blocked_address(IpAddr::V4(Ipv4Addr::new(172, 16, 0, 1))));
        assert!(is_blocked_address(IpAddr::V4(Ipv4Addr::new(172, 31, 255, 255))));
        // 192.168/16
        assert!(is_blocked_address(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))));
        // 169.254/16 — cloud metadata
        assert!(is_blocked_address(IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254))));
        // 100.64/10 — CGNAT
        assert!(is_blocked_address(IpAddr::V4(Ipv4Addr::new(100, 64, 0, 1))));
        assert!(is_blocked_address(IpAddr::V4(Ipv4Addr::new(100, 127, 255, 255))));
    }

    #[test]
    fn test_public_ip_allowed() {
        assert!(!is_blocked_address(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
        assert!(!is_blocked_address(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))));
        assert!(!is_blocked_address(IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34))));
    }

    #[test]
    fn test_ipv6_blocked() {
        // :: unspecified
        assert!(is_blocked_address(IpAddr::V6(Ipv6Addr::UNSPECIFIED)));
        // fc00::/7 unique local
        assert!(is_blocked_address("fc00::1".parse::<IpAddr>().unwrap()));
        assert!(is_blocked_address("fd00::1".parse::<IpAddr>().unwrap()));
        // fe80::/10 link-local
        assert!(is_blocked_address("fe80::1".parse::<IpAddr>().unwrap()));
    }

    #[test]
    fn test_ipv4_mapped_blocked() {
        // ::ffff:10.0.0.1 — private range embedded in IPv6
        let mapped: IpAddr = "::ffff:10.0.0.1".parse().unwrap();
        assert!(is_blocked_address(mapped));
        // ::ffff:8.8.8.8 — public
        let mapped_pub: IpAddr = "::ffff:8.8.8.8".parse().unwrap();
        assert!(!is_blocked_address(mapped_pub));
    }
}
