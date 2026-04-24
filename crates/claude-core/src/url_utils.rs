//! URL validation + redirect-safety helpers.
//!
//! Ports the URL-handling helpers from `src/tools/WebFetchTool/utils.ts`
//! so callers outside WebFetch can reuse them. The preapproved-host
//! list already lives in `claude_tools::web_fetch_preapproved`; this
//! module owns the generic parsing / validation.

use url::Url;

/// Max URL length we'll accept. 2000 chars is the PSR-approved cap —
/// the original 250-char limit was too restrictive for JWT-signed
/// URLs. See TS `MAX_URL_LENGTH` for context.
pub const MAX_URL_LENGTH: usize = 2000;

/// Validate that a URL is structurally safe to fetch. Returns `true`
/// when:
///   - Length ≤ `MAX_URL_LENGTH`
///   - Parses as an absolute URL
///   - Has no username / password (no embedded credentials)
///   - Hostname has at least two dot-separated parts (rough filter
///     for internal / non-public hostnames; `example`, `localhost`
///     and bare `api` don't pass)
///
/// Does NOT check the scheme — TS upgrades `http://` to `https://` at
/// fetch time, so this only filters structural problems.
pub fn validate_url(raw: &str) -> bool {
    if raw.len() > MAX_URL_LENGTH {
        return false;
    }
    let Ok(parsed) = Url::parse(raw) else {
        return false;
    };
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return false;
    }
    let Some(host) = parsed.host_str() else {
        return false;
    };
    host.split('.').count() >= 2
}

/// Is a redirect from `original` to `redirect_to` safe to follow?
/// Matches TS `isPermittedRedirect`:
///   - Same scheme (`http:` stays `http:`, not mixing with `https:`)
///   - Same port
///   - No credentials on the redirect target
///   - Host must match when both hosts are stripped of a leading
///     `www.` — so `example.com ↔ www.example.com` both directions
///     are allowed, but `example.com → other.example.com` is not.
pub fn is_permitted_redirect(original: &str, redirect_to: &str) -> bool {
    let Ok(o) = Url::parse(original) else {
        return false;
    };
    let Ok(r) = Url::parse(redirect_to) else {
        return false;
    };
    if o.scheme() != r.scheme() {
        return false;
    }
    if o.port() != r.port() {
        return false;
    }
    if !r.username().is_empty() || r.password().is_some() {
        return false;
    }
    let strip_www = |h: &str| h.strip_prefix("www.").unwrap_or(h).to_string();
    match (o.host_str(), r.host_str()) {
        (Some(a), Some(b)) => strip_www(a) == strip_www(b),
        _ => false,
    }
}

/// Upgrade an `http://` URL to `https://`, leaving everything else
/// unchanged. Used by WebFetch on the request path to encourage TLS
/// without forcing the model to write `https://` manually.
pub fn upgrade_http_to_https(raw: &str) -> String {
    if let Ok(mut parsed) = Url::parse(raw) {
        if parsed.scheme() == "http" {
            let _ = parsed.set_scheme("https");
            return parsed.to_string();
        }
    }
    raw.to_string()
}

/// Extract `(hostname, pathname)` from a URL string, returning `None`
/// on parse failure. Convenience shared with the preapproved-host
/// checker.
pub fn host_and_path(raw: &str) -> Option<(String, String)> {
    let u = Url::parse(raw).ok()?;
    let host = u.host_str()?.to_string();
    Some((host, u.path().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_accepts_typical_urls() {
        assert!(validate_url("https://example.com/foo"));
        assert!(validate_url("https://api.example.com"));
        assert!(validate_url("http://docs.python.org/3/"));
    }

    #[test]
    fn validate_rejects_credentials() {
        assert!(!validate_url("https://user:pass@example.com/"));
        assert!(!validate_url("https://alice@example.com/"));
    }

    #[test]
    fn validate_rejects_single_label_hostnames() {
        assert!(!validate_url("https://localhost/"));
        assert!(!validate_url("https://intranet/"));
    }

    #[test]
    fn validate_rejects_malformed() {
        assert!(!validate_url("not a url"));
        assert!(!validate_url(""));
    }

    #[test]
    fn validate_rejects_over_length() {
        let long = format!("https://example.com/{}", "a".repeat(MAX_URL_LENGTH + 1));
        assert!(!validate_url(&long));
    }

    #[test]
    fn redirect_same_host_ok() {
        assert!(is_permitted_redirect(
            "https://example.com/",
            "https://example.com/foo"
        ));
    }

    #[test]
    fn redirect_www_add_ok() {
        assert!(is_permitted_redirect(
            "https://example.com/",
            "https://www.example.com/"
        ));
    }

    #[test]
    fn redirect_www_strip_ok() {
        assert!(is_permitted_redirect(
            "https://www.example.com/",
            "https://example.com/"
        ));
    }

    #[test]
    fn redirect_scheme_change_blocked() {
        assert!(!is_permitted_redirect(
            "https://example.com/",
            "http://example.com/"
        ));
    }

    #[test]
    fn redirect_cross_domain_blocked() {
        assert!(!is_permitted_redirect(
            "https://example.com/",
            "https://other.example.com/"
        ));
        assert!(!is_permitted_redirect(
            "https://example.com/",
            "https://evil.test/"
        ));
    }

    #[test]
    fn redirect_credentials_blocked() {
        assert!(!is_permitted_redirect(
            "https://example.com/",
            "https://u:p@example.com/"
        ));
    }

    #[test]
    fn redirect_port_change_blocked() {
        assert!(!is_permitted_redirect(
            "https://example.com:443/",
            "https://example.com:8443/"
        ));
    }

    #[test]
    fn upgrade_http_to_https_flips_scheme() {
        assert_eq!(
            upgrade_http_to_https("http://example.com/foo"),
            "https://example.com/foo"
        );
    }

    #[test]
    fn upgrade_leaves_https_alone() {
        assert_eq!(
            upgrade_http_to_https("https://example.com/foo"),
            "https://example.com/foo"
        );
    }

    #[test]
    fn upgrade_leaves_non_url_alone() {
        assert_eq!(upgrade_http_to_https("not a url"), "not a url");
    }

    #[test]
    fn host_and_path_splits() {
        let (h, p) = host_and_path("https://api.example.com/v1/foo?q=1").unwrap();
        assert_eq!(h, "api.example.com");
        assert_eq!(p, "/v1/foo");
    }

    #[test]
    fn host_and_path_returns_none_on_bad_url() {
        assert!(host_and_path("not a url").is_none());
    }
}
