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
            https_proxy: read_env_pair("HTTPS_PROXY", "https_proxy"),
            http_proxy: read_env_pair("HTTP_PROXY", "http_proxy"),
            no_proxy: read_env_pair("NO_PROXY", "no_proxy"),
            ca_bundle_path: read_env_pair("SSL_CERT_FILE", "REQUESTS_CA_BUNDLE"),
        }
    }

    pub fn is_proxy_configured(&self) -> bool {
        self.https_proxy.is_some() || self.http_proxy.is_some()
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
}

pub const DEFAULT_NO_PROXY: &str = "localhost,127.0.0.1,::1,169.254.0.0/16,10.0.0.0/8,172.16.0.0/12,192.168.0.0/16,anthropic.com,.anthropic.com,*.anthropic.com,github.com,api.github.com,*.github.com,*.githubusercontent.com,registry.npmjs.org,pypi.org,files.pythonhosted.org,index.crates.io,proxy.golang.org";

fn read_env_pair(upper: &str, lower: &str) -> Option<String> {
    std::env::var(upper)
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| std::env::var(lower).ok().filter(|v| !v.is_empty()))
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
        let env = cfg.to_env_map();
        assert_eq!(env.get("HTTPS_PROXY").unwrap(), "http://proxy:8080");
        assert_eq!(env.get("https_proxy").unwrap(), "http://proxy:8080");
    }

    #[test]
    fn test_proxy_config_env_map_includes_ca() {
        let cfg = ProxyConfig {
            ca_bundle_path: Some("/tmp/ca.crt".into()),
            ..Default::default()
        };
        let env = cfg.to_env_map();
        assert_eq!(env.get("SSL_CERT_FILE").unwrap(), "/tmp/ca.crt");
        assert_eq!(env.get("CURL_CA_BUNDLE").unwrap(), "/tmp/ca.crt");
    }

    #[test]
    fn test_default_no_proxy_contains_expected_hosts() {
        assert!(DEFAULT_NO_PROXY.contains("localhost"));
        assert!(DEFAULT_NO_PROXY.contains("anthropic.com"));
        assert!(DEFAULT_NO_PROXY.contains("index.crates.io"));
    }
}
