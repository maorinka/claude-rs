//! Proxy-aware HTTP client builder.

use anyhow::{Context, Result};
use reqwest::{Client, Proxy};
use tracing;

use super::config::ProxyConfig;

/// Build a `reqwest::Client` that respects the given proxy configuration.
pub fn build_proxy_client(config: &ProxyConfig) -> Result<Client> {
    let mut builder = Client::builder();

    if let Some(ref proxy_url) = config.https_proxy {
        tracing::debug!(proxy = %proxy_url, "configuring HTTPS proxy");
        let proxy = Proxy::https(proxy_url)
            .with_context(|| format!("invalid HTTPS proxy URL: {proxy_url}"))?;
        builder = builder.proxy(proxy);
    }

    if let Some(ref proxy_url) = config.http_proxy {
        tracing::debug!(proxy = %proxy_url, "configuring HTTP proxy");
        let proxy = Proxy::http(proxy_url)
            .with_context(|| format!("invalid HTTP proxy URL: {proxy_url}"))?;
        builder = builder.proxy(proxy);
    }

    if let Some(ref no_proxy) = config.no_proxy {
        tracing::debug!(no_proxy = %no_proxy, "configuring no-proxy list");
        builder = builder.no_proxy();
        let np = no_proxy.clone();
        if let Some(ref https_url) = config.https_proxy {
            let proxy = Proxy::all(https_url)
                .with_context(|| "invalid proxy URL")?
                .no_proxy(reqwest::NoProxy::from_string(&np));
            builder = builder.proxy(proxy);
        } else if let Some(ref http_url) = config.http_proxy {
            let proxy = Proxy::all(http_url)
                .with_context(|| "invalid proxy URL")?
                .no_proxy(reqwest::NoProxy::from_string(&np));
            builder = builder.proxy(proxy);
        }
    }

    if let Some(ref ca_path) = config.ca_bundle_path {
        tracing::debug!(ca = %ca_path, "adding custom CA certificate");
        let pem = std::fs::read(ca_path)
            .with_context(|| format!("reading CA bundle: {ca_path}"))?;
        for cert in reqwest::Certificate::from_pem_bundle(&pem)
            .with_context(|| "parsing CA bundle PEM")?
        {
            builder = builder.add_root_certificate(cert);
        }
    }

    builder.build().context("building HTTP client with proxy")
}

/// Build a proxy client from the current environment.
pub fn build_proxy_client_from_env() -> Result<Client> {
    let config = ProxyConfig::from_env();
    build_proxy_client(&config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_default_client() {
        let config = ProxyConfig::default();
        assert!(build_proxy_client(&config).is_ok());
    }

    #[test]
    fn test_build_client_with_https_proxy() {
        let config = ProxyConfig {
            https_proxy: Some("http://127.0.0.1:8888".into()),
            ..Default::default()
        };
        assert!(build_proxy_client(&config).is_ok());
    }

    #[test]
    fn test_build_client_with_no_proxy() {
        let config = ProxyConfig {
            https_proxy: Some("http://proxy:3128".into()),
            no_proxy: Some("localhost,127.0.0.1".into()),
            ..Default::default()
        };
        assert!(build_proxy_client(&config).is_ok());
    }

    #[test]
    fn test_build_client_from_env() {
        assert!(build_proxy_client_from_env().is_ok());
    }
}
