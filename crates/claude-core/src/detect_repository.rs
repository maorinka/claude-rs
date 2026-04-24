//! Parse git remote URLs into host/owner/name triples.
//!
//! Port of the pure-parser half of TS `src/utils/detectRepository.ts`.
//! The runtime half (detect_current_repository / the per-cwd cache)
//! needs a git-remote lookup + cwd tracker; both live with the
//! subsystem port that hasn't landed on the Rust side yet, so this
//! patch covers the shape-level parsing only.
//!
//! Accepted remote shapes (same as TS):
//! - `git@host:owner/repo[.git]`        (SSH short form)
//! - `ssh://git@host/owner/repo[.git]`  (SSH URL)
//! - `https://host/owner/repo[.git]`    (HTTPS)
//! - `http://host/owner/repo[.git]`     (HTTP)
//! - `git://host/owner/repo[.git]`      (git protocol)
//!
//! Repo names may contain dots (e.g. `cc.kurs.web`). Host validation
//! requires a TLD made of pure ASCII letters so SSH config aliases
//! like `github.com-work` are rejected — those never resolve to a
//! real host in downstream URL construction.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedRepository {
    pub host: String,
    pub owner: String,
    pub name: String,
}

/// Parse a git remote URL / SSH spec. Returns `None` when the input
/// doesn't match any recognised shape or when the host looks like
/// an SSH config alias rather than a real domain.
pub fn parse_git_remote(input: &str) -> Option<ParsedRepository> {
    let trimmed = input.trim();

    // SSH short form: git@host:owner/repo[.git]
    if let Some(rest) = trimmed.strip_prefix("git@") {
        if let Some((host, path)) = rest.split_once(':') {
            if !host.is_empty() && !host.contains('/') {
                if let Some((owner, name)) = split_owner_name(path) {
                    if looks_like_real_hostname(host) {
                        return Some(ParsedRepository {
                            host: host.to_string(),
                            owner,
                            name,
                        });
                    }
                }
            }
        }
        return None;
    }

    // Scheme URLs: https://, http://, ssh://, git://
    let (protocol, after_scheme) = if let Some(rest) = trimmed.strip_prefix("https://") {
        ("https", rest)
    } else if let Some(rest) = trimmed.strip_prefix("http://") {
        ("http", rest)
    } else if let Some(rest) = trimmed.strip_prefix("ssh://") {
        ("ssh", rest)
    } else if let Some(rest) = trimmed.strip_prefix("git://") {
        ("git", rest)
    } else {
        return None;
    };

    // Strip optional `user@` — we ignore the user, matching TS.
    let after_userinfo = match after_scheme.find('@') {
        Some(at_idx) => {
            let user = &after_scheme[..at_idx];
            if user.is_empty() || user.contains('/') {
                after_scheme
            } else {
                &after_scheme[at_idx + 1..]
            }
        }
        None => after_scheme,
    };

    let (authority, path) = after_userinfo.split_once('/')?;
    let (host_without_port, _port) = match authority.split_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (authority, None),
    };

    if !looks_like_real_hostname(host_without_port) {
        return None;
    }

    let (owner, name) = split_owner_name(path)?;

    // Preserve the port in the host only for http/https — SSH and
    // git protocol ports aren't usable in downstream web URLs.
    let host = match protocol {
        "https" | "http" => authority.to_string(),
        _ => host_without_port.to_string(),
    };

    Some(ParsedRepository { host, owner, name })
}

/// Parse a git remote OR a plain `owner/repo` string into the
/// github.com-only `owner/repo` form. Returns `None` for any other
/// host (use `parse_git_remote` to preserve the host for GHE).
pub fn parse_github_repository(input: &str) -> Option<String> {
    let trimmed = input.trim();

    if let Some(parsed) = parse_git_remote(trimmed) {
        if parsed.host != "github.com" {
            return None;
        }
        return Some(format!("{}/{}", parsed.owner, parsed.name));
    }

    // Plain `owner/repo`: no scheme, no `@`, contains `/`.
    if !trimmed.contains("://") && !trimmed.contains('@') && trimmed.contains('/') {
        let parts: Vec<&str> = trimmed.split('/').collect();
        if parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty() {
            let repo = parts[1].strip_suffix(".git").unwrap_or(parts[1]);
            return Some(format!("{}/{}", parts[0], repo));
        }
    }

    None
}

/// True when `host` looks like a real DNS name (≥1 dot + a purely
/// alphabetic TLD). SSH config aliases such as `github.com-work`
/// fail this check because `com-work` contains a hyphen.
pub fn looks_like_real_hostname(host: &str) -> bool {
    if !host.contains('.') {
        return false;
    }
    let last = match host.rsplit('.').next() {
        Some(s) if !s.is_empty() => s,
        _ => return false,
    };
    last.chars().all(|c| c.is_ascii_alphabetic())
}

/// Split a path like `owner/repo.git` or `owner/repo` into the owner
/// and the cleaned repo name. Paths with extra segments are
/// rejected; TS uses `[^/]+/[^/]+?` anchored so more than two path
/// pieces miss the pattern entirely.
fn split_owner_name(path: &str) -> Option<(String, String)> {
    let trimmed = path.trim_start_matches('/');
    let mut parts = trimmed.splitn(3, '/');
    let owner = parts.next()?;
    let name = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    if owner.is_empty() || name.is_empty() {
        return None;
    }
    let clean_name = name.strip_suffix(".git").unwrap_or(name);
    if clean_name.is_empty() {
        return None;
    }
    Some((owner.to_string(), clean_name.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ssh_short_form() {
        let p = parse_git_remote("git@github.com:owner/repo.git").unwrap();
        assert_eq!(p.host, "github.com");
        assert_eq!(p.owner, "owner");
        assert_eq!(p.name, "repo");
    }

    #[test]
    fn parses_ssh_short_form_without_dot_git() {
        let p = parse_git_remote("git@github.com:o/r").unwrap();
        assert_eq!(p.name, "r");
    }

    #[test]
    fn parses_https_url() {
        let p = parse_git_remote("https://github.com/owner/repo.git").unwrap();
        assert_eq!(p.host, "github.com");
        assert_eq!(p.owner, "owner");
        assert_eq!(p.name, "repo");
    }

    #[test]
    fn parses_http_url() {
        let p = parse_git_remote("http://host.io/o/r").unwrap();
        assert_eq!(p.host, "host.io");
    }

    #[test]
    fn parses_ssh_url_with_userinfo() {
        let p = parse_git_remote("ssh://git@ghe.corp.com/owner/repo.git").unwrap();
        assert_eq!(p.host, "ghe.corp.com");
        assert_eq!(p.owner, "owner");
        assert_eq!(p.name, "repo");
    }

    #[test]
    fn parses_git_protocol() {
        let p = parse_git_remote("git://host.io/o/r.git").unwrap();
        assert_eq!(p.host, "host.io");
    }

    #[test]
    fn preserves_port_on_https_only() {
        let https = parse_git_remote("https://host.io:8443/o/r").unwrap();
        assert_eq!(https.host, "host.io:8443");
        let ssh = parse_git_remote("ssh://git@host.io:2222/o/r.git").unwrap();
        assert_eq!(ssh.host, "host.io");
    }

    #[test]
    fn handles_repo_names_with_dots() {
        let p = parse_git_remote("https://github.com/acme/cc.kurs.web").unwrap();
        assert_eq!(p.name, "cc.kurs.web");
    }

    #[test]
    fn rejects_hostname_without_real_tld() {
        // SSH alias form.
        assert!(parse_git_remote("git@github.com-work:o/r.git").is_none());
        // Hyphenated last segment.
        assert!(parse_git_remote("https://alias-host/o/r.git").is_none());
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_git_remote("not a url").is_none());
        assert!(parse_git_remote("").is_none());
        assert!(parse_git_remote("https://").is_none());
    }

    #[test]
    fn rejects_extra_path_segments() {
        assert!(parse_git_remote("https://host.io/o/r/extra").is_none());
        assert!(parse_git_remote("git@host.io:o/r/extra.git").is_none());
    }

    #[test]
    fn parse_github_repository_returns_pair_for_github() {
        assert_eq!(
            parse_github_repository("https://github.com/o/r.git").as_deref(),
            Some("o/r")
        );
        assert_eq!(
            parse_github_repository("git@github.com:o/r.git").as_deref(),
            Some("o/r")
        );
    }

    #[test]
    fn parse_github_repository_rejects_ghe() {
        assert!(parse_github_repository("https://ghe.corp.com/o/r.git").is_none());
    }

    #[test]
    fn parse_github_repository_accepts_plain_pair() {
        assert_eq!(
            parse_github_repository("acme/widgets").as_deref(),
            Some("acme/widgets")
        );
        assert_eq!(
            parse_github_repository("acme/widgets.git").as_deref(),
            Some("acme/widgets")
        );
    }

    #[test]
    fn parse_github_repository_rejects_malformed_plain_pair() {
        assert!(parse_github_repository("no-slash").is_none());
        assert!(parse_github_repository("/").is_none());
        assert!(parse_github_repository("a/b/c").is_none());
    }

    #[test]
    fn real_hostname_accepts_real_domain() {
        assert!(looks_like_real_hostname("github.com"));
        assert!(looks_like_real_hostname("ghe.corp.io"));
    }

    #[test]
    fn real_hostname_rejects_alias_like() {
        assert!(!looks_like_real_hostname("github.com-work"));
        assert!(!looks_like_real_hostname("nolocal"));
        assert!(!looks_like_real_hostname("host.123"));
        assert!(!looks_like_real_hostname(""));
    }
}
