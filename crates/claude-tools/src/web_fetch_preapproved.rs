//! Preapproved hosts for WebFetch.
//!
//! Port of `src/tools/WebFetchTool/preapproved.ts`. Each entry is either a
//! bare hostname (matched exactly) or `host/path-prefix` (path must match
//! exactly or continue with `/`). O(1) lookup via a HashSet for bare hosts,
//! small linear scan for the handful of path-scoped entries.
//!
//! SECURITY WARNING (from TS): these entries are ONLY for WebFetch (read-only
//! GET). The sandbox network allowlist must NOT inherit this list — some of
//! these sites (huggingface.co, kaggle.com, nuget.org) accept uploads.

use once_cell::sync::Lazy;
use std::collections::{HashMap, HashSet};

const RAW_ENTRIES: &[&str] = &[
    // Anthropic
    "platform.claude.com",
    "code.claude.com",
    "modelcontextprotocol.io",
    "github.com/anthropics",
    "agentskills.io",
    // Top programming languages
    "docs.python.org",
    "en.cppreference.com",
    "docs.oracle.com",
    "learn.microsoft.com",
    "developer.mozilla.org",
    "go.dev",
    "pkg.go.dev",
    "www.php.net",
    "docs.swift.org",
    "kotlinlang.org",
    "ruby-doc.org",
    "doc.rust-lang.org",
    "www.typescriptlang.org",
    // Web & JS frameworks
    "react.dev",
    "angular.io",
    "vuejs.org",
    "nextjs.org",
    "expressjs.com",
    "nodejs.org",
    "bun.sh",
    "jquery.com",
    "getbootstrap.com",
    "tailwindcss.com",
    "d3js.org",
    "threejs.org",
    "redux.js.org",
    "webpack.js.org",
    "jestjs.io",
    "reactrouter.com",
    // Python frameworks
    "docs.djangoproject.com",
    "flask.palletsprojects.com",
    "fastapi.tiangolo.com",
    "pandas.pydata.org",
    "numpy.org",
    "www.tensorflow.org",
    "pytorch.org",
    "scikit-learn.org",
    "matplotlib.org",
    "requests.readthedocs.io",
    "jupyter.org",
    // PHP
    "laravel.com",
    "symfony.com",
    "wordpress.org",
    // Java
    "docs.spring.io",
    "hibernate.org",
    "tomcat.apache.org",
    "gradle.org",
    "maven.apache.org",
    // .NET
    "asp.net",
    "dotnet.microsoft.com",
    "nuget.org",
    "blazor.net",
    // Mobile
    "reactnative.dev",
    "docs.flutter.dev",
    "developer.apple.com",
    "developer.android.com",
    // Data science
    "keras.io",
    "spark.apache.org",
    "huggingface.co",
    "www.kaggle.com",
    // Databases
    "www.mongodb.com",
    "redis.io",
    "www.postgresql.org",
    "dev.mysql.com",
    "www.sqlite.org",
    "graphql.org",
    "prisma.io",
    // Cloud & DevOps
    "docs.aws.amazon.com",
    "cloud.google.com",
    "kubernetes.io",
    "www.docker.com",
    "www.terraform.io",
    "www.ansible.com",
    "vercel.com/docs",
    "docs.netlify.com",
    "devcenter.heroku.com",
    // Testing & monitoring
    "cypress.io",
    "selenium.dev",
    // Game development
    "docs.unity.com",
    "docs.unrealengine.com",
    // Other essentials
    "git-scm.com",
    "nginx.org",
    "httpd.apache.org",
];

struct Tables {
    hostname_only: HashSet<&'static str>,
    path_prefixes: HashMap<&'static str, Vec<&'static str>>,
}

static TABLES: Lazy<Tables> = Lazy::new(|| {
    let mut hosts: HashSet<&'static str> = HashSet::new();
    let mut paths: HashMap<&'static str, Vec<&'static str>> = HashMap::new();
    for &entry in RAW_ENTRIES {
        if let Some(slash) = entry.find('/') {
            let (host, path) = entry.split_at(slash);
            paths.entry(host).or_default().push(path);
        } else {
            hosts.insert(entry);
        }
    }
    Tables {
        hostname_only: hosts,
        path_prefixes: paths,
    }
});

/// Check whether `(hostname, pathname)` matches a preapproved entry.
/// Path prefixes enforce segment boundaries: `/anthropics` does not match
/// `/anthropics-evil/malware`. Mirrors the TS `isPreapprovedHost`.
pub fn is_preapproved_host(hostname: &str, pathname: &str) -> bool {
    if TABLES.hostname_only.contains(hostname) {
        return true;
    }
    if let Some(prefixes) = TABLES.path_prefixes.get(hostname) {
        for p in prefixes {
            if pathname == *p || pathname.starts_with(&format!("{}/", p)) {
                return true;
            }
        }
    }
    false
}

/// Parse a URL string and check whether its host+path are preapproved.
pub fn is_preapproved_url(url: &str) -> bool {
    // Minimal-dep parse: reqwest already ships url in its graph so this is
    // free. Guard against malformed input.
    match reqwest::Url::parse(url) {
        Ok(u) => {
            let host = u.host_str().unwrap_or("");
            is_preapproved_host(host, u.path())
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_hostname_matches() {
        assert!(is_preapproved_host("docs.python.org", "/3/library/os.html"));
        assert!(is_preapproved_host("react.dev", "/"));
    }

    #[test]
    fn path_scoped_matches() {
        assert!(is_preapproved_host("github.com", "/anthropics"));
        assert!(is_preapproved_host("github.com", "/anthropics/claude-code"));
    }

    #[test]
    fn path_scoped_boundaries_enforced() {
        // must NOT match — no slash boundary after "anthropics"
        assert!(!is_preapproved_host(
            "github.com",
            "/anthropics-evil/malware"
        ));
        // and the bare host itself isn't approved
        assert!(!is_preapproved_host("github.com", "/torvalds/linux"));
    }

    #[test]
    fn unknown_hostname_rejected() {
        assert!(!is_preapproved_host("evil.example", "/"));
    }

    #[test]
    fn url_form_works() {
        assert!(is_preapproved_url("https://docs.python.org/3/"));
        assert!(!is_preapproved_url("https://evil.example/"));
    }
}
