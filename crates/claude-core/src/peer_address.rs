//! Parse peer addresses for inter-process messaging.
//!
//! Port of TS `src/utils/peerAddress.ts`. Kept separate from the
//! full peer registry so tool enumeration can import just the
//! parser without pulling bridge + UDS modules.
//!
//! Address shapes:
//! - `uds:PATH`                  → Uds { target: PATH }
//! - `bridge:TARGET`             → Bridge { target: TARGET }
//! - `/abs/path` (bare path)     → Uds { target: "/abs/path" }
//!   Legacy form — old UDS senders emit bare socket paths in `from`;
//!   routing them through the UDS branch keeps replies from being
//!   silently dropped into teammate routing.
//! - anything else                → Other { target: AS-IS }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerAddressScheme {
    Uds,
    Bridge,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedAddress {
    pub scheme: PeerAddressScheme,
    pub target: String,
}

/// Parse a peer address into its scheme + target. See module-level
/// docs for the accepted shapes.
pub fn parse_address(to: &str) -> ParsedAddress {
    if let Some(rest) = to.strip_prefix("uds:") {
        return ParsedAddress {
            scheme: PeerAddressScheme::Uds,
            target: rest.to_string(),
        };
    }
    if let Some(rest) = to.strip_prefix("bridge:") {
        return ParsedAddress {
            scheme: PeerAddressScheme::Bridge,
            target: rest.to_string(),
        };
    }
    if to.starts_with('/') {
        return ParsedAddress {
            scheme: PeerAddressScheme::Uds,
            target: to.to_string(),
        };
    }
    ParsedAddress {
        scheme: PeerAddressScheme::Other,
        target: to.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uds_scheme_strips_prefix() {
        let p = parse_address("uds:/tmp/sock");
        assert_eq!(p.scheme, PeerAddressScheme::Uds);
        assert_eq!(p.target, "/tmp/sock");
    }

    #[test]
    fn bridge_scheme_strips_prefix() {
        let p = parse_address("bridge:abc-123");
        assert_eq!(p.scheme, PeerAddressScheme::Bridge);
        assert_eq!(p.target, "abc-123");
    }

    #[test]
    fn bare_absolute_path_is_legacy_uds() {
        let p = parse_address("/var/run/x.sock");
        assert_eq!(p.scheme, PeerAddressScheme::Uds);
        assert_eq!(p.target, "/var/run/x.sock");
    }

    #[test]
    fn plain_name_falls_through_to_other() {
        let p = parse_address("session_manager");
        assert_eq!(p.scheme, PeerAddressScheme::Other);
        assert_eq!(p.target, "session_manager");
    }

    #[test]
    fn empty_string_is_other_empty() {
        let p = parse_address("");
        assert_eq!(p.scheme, PeerAddressScheme::Other);
        assert_eq!(p.target, "");
    }

    #[test]
    fn scheme_is_case_sensitive() {
        // TS used startsWith; Rust equivalents match exactly.
        let p = parse_address("UDS:/x");
        assert_eq!(p.scheme, PeerAddressScheme::Other);
        assert_eq!(p.target, "UDS:/x");
    }
}
