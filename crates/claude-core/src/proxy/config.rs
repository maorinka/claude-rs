//! Proxy configuration from environment variables.

use std::collections::HashMap;

/// Proxy settings extracted from the environment.
#[derive(Debug, Clone, Default)]
pub struct ProxyConfig {
    pub https_proxy: Option<String>,
    pub http_proxy: Option<String>,
    pub no_proxy: Option<String>,
    pub ca_bundle_path: Option<String>,
}

impl ProxyConfig {
    pub fn from_env() -> Self {
        Self {
            https_proxy: read_env_pair("https_proxy", "HTTPS_PROXY"),
            http_proxy: read_env_pair("http_proxy", "HTTP_PROXY"),
            no_proxy: read_env_pair("no_proxy", "NO_PROXY"),
            ca_bundle_path: read_env_any(&[
                "NODE_EXTRA_CA_CERTS",
                "SSL_CERT_FILE",
                "REQUESTS_CA_BUNDLE",
                "CURL_CA_BUNDLE",
            ]),
        }
    }

    pub fn is_proxy_configured(&self) -> bool {
        self.https_proxy.is_some() || self.http_proxy.is_some()
    }

    /// TS `getProxyUrl` parity: one active proxy URL, preferring
    /// `https_proxy > HTTPS_PROXY > http_proxy > HTTP_PROXY`.
    pub fn active_proxy_url(&self) -> Option<&str> {
        self.https_proxy.as_deref().or(self.http_proxy.as_deref())
    }

    pub fn to_env_map(&self) -> HashMap<String, String> {
        let mut env = HashMap::new();
        if let Some(ref url) = self.https_proxy {
            env.insert("HTTPS_PROXY".into(), url.clone());
            env.insert("https_proxy".into(), url.clone());
        }
        if let Some(ref url) = self.http_proxy {
            env.insert("HTTP_PROXY".into(), url.clone());
            env.insert("http_proxy".into(), url.clone());
        }
        if let Some(ref no) = self.no_proxy {
            env.insert("NO_PROXY".into(), no.clone());
            env.insert("no_proxy".into(), no.clone());
        }
        if let Some(ref ca) = self.ca_bundle_path {
            env.insert("SSL_CERT_FILE".into(), ca.clone());
            env.insert("NODE_EXTRA_CA_CERTS".into(), ca.clone());
            env.insert("REQUESTS_CA_BUNDLE".into(), ca.clone());
            env.insert("CURL_CA_BUNDLE".into(), ca.clone());
        }
        env
    }

    /// TS `shouldBypassProxy` parity for callers that need to decide before
    /// constructing a request/transport, especially WebSocket paths.
    pub fn should_bypass_proxy(&self, url: &str) -> bool {
        should_bypass_proxy(url, self.no_proxy.as_deref())
    }

    /// TS `getWebSocketProxyUrl` parity: return the active proxy unless the
    /// destination is covered by NO_PROXY.
    pub fn websocket_proxy_url(&self, url: &str) -> Option<&str> {
        let proxy = self.active_proxy_url()?;
        (!self.should_bypass_proxy(url)).then_some(proxy)
    }
}

pub const DEFAULT_NO_PROXY: &str = "localhost,127.0.0.1,::1,169.254.0.0/16,10.0.0.0/8,172.16.0.0/12,192.168.0.0/16,anthropic.com,.anthropic.com,*.anthropic.com,github.com,api.github.com,*.github.com,*.githubusercontent.com,registry.npmjs.org,pypi.org,files.pythonhosted.org,index.crates.io,proxy.golang.org";

fn read_env_pair(preferred: &str, fallback: &str) -> Option<String> {
    std::env::var(preferred)
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| std::env::var(fallback).ok().filter(|v| !v.is_empty()))
}

fn read_env_any(names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|name| std::env::var(name).ok().filter(|v| !v.is_empty()))
}

/// TS `shouldBypassProxy` parity.
///
/// Supports wildcard `*`, comma/space-separated entries, exact host/IP matches,
/// leading-dot domain suffixes, and port-specific `host:port` entries. Invalid
/// URLs do not bypass.
pub fn should_bypass_proxy(url_string: &str, no_proxy: Option<&str>) -> bool {
    let Some(no_proxy) = no_proxy.filter(|value| !value.is_empty()) else {
        return false;
    };
    if no_proxy == "*" {
        return true;
    }

    let Ok(url) = url::Url::parse(url_string) else {
        return false;
    };
    let Some(hostname) = url.host_str().map(|host| host.to_ascii_lowercase()) else {
        return false;
    };
    let port =
        url.port_or_known_default()
            .unwrap_or_else(|| if url.scheme() == "https" { 443 } else { 80 });
    let host_with_port = format!("{hostname}:{port}");

    no_proxy
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|pattern| !pattern.is_empty())
        .any(|pattern| {
            let pattern = pattern.trim().to_ascii_lowercase();
            if pattern.contains(':') {
                return host_with_port == pattern;
            }
            if let Some(suffix) = pattern.strip_prefix('.') {
                return hostname == suffix || hostname.ends_with(&pattern);
            }
            hostname == pattern
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_proxy_config_default() {
        let cfg = ProxyConfig::default();
        assert!(!cfg.is_proxy_configured());
        assert!(cfg.to_env_map().is_empty());
    }

    #[test]
    fn test_proxy_config_with_https() {
        let cfg = ProxyConfig {
            https_proxy: Some("http://proxy:8080".into()),
            ..Default::default()
        };
        assert!(cfg.is_proxy_configured());
        assert_eq!(cfg.active_proxy_url(), Some("http://proxy:8080"));
        let env = cfg.to_env_map();
        assert_eq!(env.get("HTTPS_PROXY").unwrap(), "http://proxy:8080");
        assert_eq!(env.get("https_proxy").unwrap(), "http://proxy:8080");
    }

    #[test]
    fn test_active_proxy_url_prefers_https_over_http() {
        let cfg = ProxyConfig {
            https_proxy: Some("http://https-proxy:8080".into()),
            http_proxy: Some("http://http-proxy:8080".into()),
            ..Default::default()
        };
        assert_eq!(cfg.active_proxy_url(), Some("http://https-proxy:8080"));
    }

    #[test]
    fn test_proxy_config_env_map_includes_ca() {
        let cfg = ProxyConfig {
            ca_bundle_path: Some("/tmp/ca.crt".into()),
            ..Default::default()
        };
        let env = cfg.to_env_map();
        assert_eq!(env.get("SSL_CERT_FILE").unwrap(), "/tmp/ca.crt");
        assert_eq!(env.get("NODE_EXTRA_CA_CERTS").unwrap(), "/tmp/ca.crt");
        assert_eq!(env.get("CURL_CA_BUNDLE").unwrap(), "/tmp/ca.crt");
    }

    #[test]
    fn test_default_no_proxy_contains_expected_hosts() {
        assert!(DEFAULT_NO_PROXY.contains("localhost"));
        assert!(DEFAULT_NO_PROXY.contains("anthropic.com"));
        assert!(DEFAULT_NO_PROXY.contains("index.crates.io"));
    }

    #[test]
    fn should_bypass_proxy_matches_ts_rules() {
        assert!(!should_bypass_proxy("https://example.com", None));
        assert!(should_bypass_proxy("https://example.com", Some("*")));
        assert!(should_bypass_proxy(
            "https://localhost:8443/path",
            Some("localhost")
        ));
        assert!(should_bypass_proxy(
            "https://api.example.com",
            Some(".example.com")
        ));
        assert!(should_bypass_proxy(
            "https://example.com",
            Some(".example.com")
        ));
        assert!(!should_bypass_proxy(
            "https://notexample.com",
            Some(".example.com")
        ));
        assert!(should_bypass_proxy(
            "https://example.com:8443",
            Some("example.com:8443")
        ));
        assert!(!should_bypass_proxy(
            "https://example.com:443",
            Some("example.com:8443")
        ));
        assert!(should_bypass_proxy(
            "http://127.0.0.1:8080",
            Some("localhost, 127.0.0.1")
        ));
        assert!(should_bypass_proxy("not a url", Some("*")));
        assert!(!should_bypass_proxy("not a url", Some("localhost")));
    }

    #[test]
    fn websocket_proxy_url_respects_no_proxy() {
        let cfg = ProxyConfig {
            https_proxy: Some("http://proxy:8080".into()),
            no_proxy: Some("localhost,.internal".into()),
            ..Default::default()
        };
        assert_eq!(
            cfg.websocket_proxy_url("wss://api.example.com/session"),
            Some("http://proxy:8080")
        );
        assert_eq!(cfg.websocket_proxy_url("ws://localhost:3000"), None);
        assert_eq!(
            cfg.websocket_proxy_url("wss://worker.service.internal/ws"),
            None
        );
    }
}
