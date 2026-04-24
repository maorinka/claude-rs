//! Cross-platform "open URL / path in OS default handler" with URL
//! protocol validation.
//!
//! Port of TS `utils/browser.ts:1-68`.
//!
//! The Rust ecosystem's `open` crate already handles the macOS / Linux /
//! Windows dispatch (equivalent to `open` / `xdg-open` / `rundll32`),
//! so this module is a thin wrapper whose value is the **URL protocol
//! allow-list** — `http://` / `https://` only. Without that guard,
//! [`open_browser`] would hand `javascript:`, `file://`, or
//! `data:` URIs to the OS handler, which on some platforms would
//! execute them in unexpected contexts (e.g. default browser auto-
//! running `javascript:` URLs inside whatever page is active).
//!
//! [`open_path`] is a filesystem-path variant with no protocol check —
//! the underlying `open::that` treats a relative path as a path, not a
//! URL, so the JS-URI smuggling concern doesn't apply.

use thiserror::Error;
use url::Url;

#[derive(Debug, Error)]
pub enum UrlValidationError {
    #[error("Invalid URL format: {url}")]
    InvalidFormat { url: String },
    #[error("Invalid URL protocol: must use http:// or https://, got {protocol}:")]
    UnsupportedProtocol { protocol: String },
}

fn validate_url(url: &str) -> Result<Url, UrlValidationError> {
    let parsed = Url::parse(url).map_err(|_| UrlValidationError::InvalidFormat {
        url: url.to_owned(),
    })?;
    match parsed.scheme() {
        "http" | "https" => Ok(parsed),
        other => Err(UrlValidationError::UnsupportedProtocol {
            protocol: other.to_owned(),
        }),
    }
}

/// Open a filesystem path using the OS's default handler. TS's logic
/// (platform-branch on `open` / `explorer` / `xdg-open`) is handled
/// inside the `open` crate.
///
/// Returns `true` if the handler launched cleanly, `false` otherwise.
/// TS's equivalent swallows any error into `false` for the same
/// fire-and-forget semantics (`browser.ts:34-36`).
pub fn open_path(path: &str) -> bool {
    open::that(path).is_ok()
}

/// Open a URL in the user's browser after validating the protocol.
///
/// Returns `Ok(true)` / `Ok(false)` if the URL passed validation
/// (matching TS's bool return: success vs. launcher failure). Returns
/// `Err(UrlValidationError)` when the protocol is non-http/https — TS
/// throws in that branch (`browser.ts:14-17`); Rust surfaces the
/// specific reason as a typed error so callers can log or surface it.
pub fn open_browser(url: &str) -> Result<bool, UrlValidationError> {
    let parsed = validate_url(url)?;
    Ok(open::that(parsed.as_str()).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_accepts_http() {
        assert!(validate_url("http://example.com/path").is_ok());
    }

    #[test]
    fn validate_accepts_https() {
        assert!(validate_url("https://example.com/path?q=1").is_ok());
    }

    #[test]
    fn validate_rejects_file_scheme() {
        // The security-critical case — TS comment calls out protocol
        // validation specifically to block this.
        let err = validate_url("file:///etc/passwd").unwrap_err();
        assert!(
            matches!(err, UrlValidationError::UnsupportedProtocol { ref protocol } if protocol == "file"),
            "expected file-protocol error, got {err:?}"
        );
    }

    #[test]
    fn validate_rejects_javascript_scheme() {
        let err = validate_url("javascript:alert(1)").unwrap_err();
        assert!(matches!(
            err,
            UrlValidationError::UnsupportedProtocol { ref protocol } if protocol == "javascript"
        ));
    }

    #[test]
    fn validate_rejects_data_scheme() {
        let err = validate_url("data:text/html,<script>alert(1)</script>").unwrap_err();
        assert!(matches!(
            err,
            UrlValidationError::UnsupportedProtocol { ref protocol } if protocol == "data"
        ));
    }

    #[test]
    fn validate_rejects_bare_string() {
        let err = validate_url("not a url").unwrap_err();
        assert!(matches!(err, UrlValidationError::InvalidFormat { .. }));
    }

    #[test]
    fn validate_rejects_empty_string() {
        let err = validate_url("").unwrap_err();
        assert!(matches!(err, UrlValidationError::InvalidFormat { .. }));
    }

    #[test]
    fn open_browser_rejects_non_http_url_without_dispatching() {
        // If validation fails, `open::that` is NEVER called — there's
        // no way to observe that from outside, so the test relies on
        // the error variant matching. No side effects on the host.
        let err = open_browser("file:///tmp/x").unwrap_err();
        assert!(matches!(
            err,
            UrlValidationError::UnsupportedProtocol { .. }
        ));
    }
}
