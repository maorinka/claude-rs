//! Undercover mode: ant-only safety for public-repo contributions.
//!
//! Port of TS `src/utils/undercover.ts` + the repo-classification
//! cache from `src/utils/commitAttribution.ts`.
//!
//! When ACTIVE, undercover mode adds safety instructions to commit
//! and PR prompts and strips attribution so Anthropic-internal
//! codenames, project names, unreleased model versions, and
//! Claude-authored markers never leak into a public commit history.
//!
//! ## Activation (matches TS exactly)
//!
//! | `USER_TYPE` | `CLAUDE_CODE_UNDERCOVER` | Repo class        | Active? |
//! |-------------|--------------------------|-------------------|---------|
//! | not `ant`   | any                      | any               | **no**  |
//! | `ant`       | truthy                   | any               | **yes** |
//! | `ant`       | unset/falsy              | `Internal`        | no      |
//! | `ant`       | unset/falsy              | `External`/`None` | **yes** |
//! | `ant`       | unset/falsy              | unprimed (None)   | **yes** |
//!
//! There is **no force-OFF**. When the repo-class cache is unprimed,
//! the conservative default is undercover ON — we'd rather
//! over-apply the safety net than miss a codename leak.
//!
//! ## Repo classification
//!
//! Primed once per process by walking `git remote get-url origin`
//! (in the current cwd) and checking whether the URL contains any
//! of the [`INTERNAL_MODEL_REPOS`] allowlist entries. TS uses
//! `remoteUrl.includes(repo)` (substring match) — NOT parsed
//! owner/repo equality — so the Rust port does the same. Priming
//! is lazy at first call; callers that want TS's "settle before
//! first prompt" behaviour should invoke
//! [`prime_repo_class_from_remote`] at startup.

use crate::errors_util::is_env_truthy;
use crate::user_type;
use std::sync::RwLock;

/// Classification of the current git repo's origin remote.
///
/// | Variant    | Meaning                                         |
/// |------------|-------------------------------------------------|
/// | `Internal` | Remote URL contains an [`INTERNAL_MODEL_REPOS`] entry. |
/// | `External` | Has a remote URL, but not on the allowlist.     |
/// | `None`     | No remote configured, or not a git repo.        |
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoClass {
    Internal,
    External,
    None,
}

static REPO_CLASS_CACHE: RwLock<Option<RepoClass>> = RwLock::new(None);

/// Peek at the cached classification without priming. Returns
/// `None` if the check hasn't run yet. Matches TS
/// [`getRepoClassCached`](claude-code-leaked/src/utils/commitAttribution.ts).
pub fn get_repo_class_cached() -> Option<RepoClass> {
    *REPO_CLASS_CACHE.read().unwrap_or_else(|p| p.into_inner())
}

/// Returns `true` only if the cache has been primed AND the repo
/// was classified as `Internal`. Matches TS
/// [`isInternalModelRepoCached`](claude-code-leaked/src/utils/commitAttribution.ts):
/// safe default `false` when unprimed.
pub fn is_internal_repo_cached() -> bool {
    matches!(get_repo_class_cached(), Some(RepoClass::Internal))
}

/// Prime the cache with an explicit classification. Use for:
/// - tests that need deterministic state,
/// - callers that have already resolved the remote via a different
///   path (e.g. the agent-context worktree override).
///
/// Idempotent beyond the first call — TS primes once and keeps
/// the first result for the process lifetime; Rust mirrors that.
pub fn set_repo_class_for_test_or_prime(class: RepoClass) {
    let mut guard = REPO_CLASS_CACHE.write().unwrap_or_else(|p| p.into_inner());
    if guard.is_none() {
        *guard = Some(class);
    }
}

/// Test-only: force-overwrite the cache and re-prime. TS has no
/// equivalent — its cache is module-scoped and process-lifetime.
/// Rust exposes this so tests can transition between states.
#[cfg(test)]
pub(crate) fn reset_repo_class_cache() {
    let mut guard = REPO_CLASS_CACHE.write().unwrap_or_else(|p| p.into_inner());
    *guard = None;
}

/// Classify a remote URL against [`INTERNAL_MODEL_REPOS`]. Pure —
/// exposed for testing and for callers that already have the remote
/// in hand (e.g. from a pre-existing git query).
///
/// Substring semantics match TS `remoteUrl.includes(repo)` exactly.
/// Case-sensitive, no URL parsing — a URL with an unusual prefix
/// that still contains `"github.com:anthropics/apps"` as a substring
/// will match.
///
/// Empty-string URL is treated as [`RepoClass::None`] to match TS's
/// JS-truthy gate (`if (!remoteUrl) { return 'none' }` at
/// `commitAttribution.ts:122`). `None` and `Some("")` are both
/// "no remote configured".
pub fn classify_remote_url(remote_url: Option<&str>) -> RepoClass {
    let Some(url) = remote_url else {
        return RepoClass::None;
    };
    if url.is_empty() {
        return RepoClass::None;
    }
    if INTERNAL_MODEL_REPOS.iter().any(|repo| url.contains(repo)) {
        RepoClass::Internal
    } else {
        RepoClass::External
    }
}

/// Prime the cache by running `git remote get-url origin` in `cwd`
/// and classifying the output. First call wins — subsequent calls
/// see the cached value and no-op. Errors (no git, no origin) are
/// treated as `RepoClass::None`.
///
/// Safe to call unconditionally at startup; cost is one subprocess
/// per process-lifetime.
pub fn prime_repo_class_from_remote(cwd: &std::path::Path) -> RepoClass {
    if let Some(existing) = get_repo_class_cached() {
        return existing;
    }
    let output = std::process::Command::new("git")
        .args(["-C"])
        .arg(cwd)
        .args(["remote", "get-url", "origin"])
        .output();

    let remote = match output {
        Ok(o) if o.status.success() => {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if s.is_empty() {
                None
            } else {
                Some(s)
            }
        }
        _ => None,
    };

    let class = classify_remote_url(remote.as_deref());
    set_repo_class_for_test_or_prime(class);
    class
}

/// Main undercover predicate. See the module docs for the full
/// activation table. Matches TS
/// [`isUndercover`](claude-code-leaked/src/utils/undercover.ts)
/// exactly.
pub fn is_undercover() -> bool {
    if !user_type::is_ant() {
        return false;
    }
    if is_env_truthy("CLAUDE_CODE_UNDERCOVER") {
        return true;
    }
    // Auto: active unless we've positively confirmed `Internal`.
    // Unprimed (`None`), `None` (no remote), and `External` all
    // → undercover ON.
    !matches!(get_repo_class_cached(), Some(RepoClass::Internal))
}

/// The safety-instructions block spliced into commit + PR prompts
/// when callers want to warn the model against leaking internal
/// information. Ant-only — returns `""` for every other user type,
/// matching TS
/// [`getUndercoverInstructions`](claude-code-leaked/src/utils/undercover.ts).
///
/// TS callers separately gate on `isUndercover()` before reading
/// this — the function itself does not. The Rust port preserves
/// that separation so future callers can mix-and-match.
pub fn get_undercover_instructions() -> &'static str {
    if user_type::is_ant() {
        UNDERCOVER_INSTRUCTIONS
    } else {
        ""
    }
}

const UNDERCOVER_INSTRUCTIONS: &str = r#"## UNDERCOVER MODE — CRITICAL

You are operating UNDERCOVER in a PUBLIC/OPEN-SOURCE repository. Your commit
messages, PR titles, and PR bodies MUST NOT contain ANY Anthropic-internal
information. Do not blow your cover.

NEVER include in commit messages or PR descriptions:
- Internal model codenames (animal names like Capybara, Tengu, etc.)
- Unreleased model version numbers (e.g., opus-4-7, sonnet-4-8)
- Internal repo or project names (e.g., claude-cli-internal, anthropics/…)
- Internal tooling, Slack channels, or short links (e.g., go/cc, #claude-code-…)
- The phrase "Claude Code" or any mention that you are an AI
- Any hint of what model or version you are
- Co-Authored-By lines or any other attribution

Write commit messages as a human developer would — describe only what the code
change does.

GOOD:
- "Fix race condition in file watcher initialization"
- "Add support for custom key bindings"
- "Refactor parser for better error messages"

BAD (never write these):
- "Fix bug found while testing with Claude Capybara"
- "1-shotted by claude-opus-4-6"
- "Generated with Claude Code"
- "Co-Authored-By: Claude Opus 4.6 <…>"
"#;

/// Repositories that trigger `RepoClass::Internal`. Verbatim port
/// of the TS list in
/// [`commitAttribution.ts`](claude-code-leaked/src/utils/commitAttribution.ts) —
/// 22 repos × 2 URL formats (SSH `github.com:org/name` and HTTPS
/// `github.com/org/name`) = 44 entries. Substring-matched via
/// `remote_url.contains(repo)` — callers do NOT parse URLs. Adding
/// a new allowlisted repo requires both formats to stay consistent.
pub const INTERNAL_MODEL_REPOS: &[&str] = &[
    "github.com:anthropics/claude-cli-internal",
    "github.com/anthropics/claude-cli-internal",
    "github.com:anthropics/anthropic",
    "github.com/anthropics/anthropic",
    "github.com:anthropics/apps",
    "github.com/anthropics/apps",
    "github.com:anthropics/casino",
    "github.com/anthropics/casino",
    "github.com:anthropics/dbt",
    "github.com/anthropics/dbt",
    "github.com:anthropics/dotfiles",
    "github.com/anthropics/dotfiles",
    "github.com:anthropics/terraform-config",
    "github.com/anthropics/terraform-config",
    "github.com:anthropics/hex-export",
    "github.com/anthropics/hex-export",
    "github.com:anthropics/feedback-v2",
    "github.com/anthropics/feedback-v2",
    "github.com:anthropics/labs",
    "github.com/anthropics/labs",
    "github.com:anthropics/argo-rollouts",
    "github.com/anthropics/argo-rollouts",
    "github.com:anthropics/starling-configs",
    "github.com/anthropics/starling-configs",
    "github.com:anthropics/ts-tools",
    "github.com/anthropics/ts-tools",
    "github.com:anthropics/ts-capsules",
    "github.com/anthropics/ts-capsules",
    "github.com:anthropics/feldspar-testing",
    "github.com/anthropics/feldspar-testing",
    "github.com:anthropics/trellis",
    "github.com/anthropics/trellis",
    "github.com:anthropics/claude-for-hiring",
    "github.com/anthropics/claude-for-hiring",
    "github.com:anthropics/forge-web",
    "github.com/anthropics/forge-web",
    "github.com:anthropics/infra-manifests",
    "github.com/anthropics/infra-manifests",
    "github.com:anthropics/mycro_manifests",
    "github.com/anthropics/mycro_manifests",
    "github.com:anthropics/mycro_configs",
    "github.com/anthropics/mycro_configs",
    "github.com:anthropics/mobile-apps",
    "github.com/anthropics/mobile-apps",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::ENV_LOCK;

    #[test]
    fn allowlist_has_44_entries() {
        assert_eq!(INTERNAL_MODEL_REPOS.len(), 44);
    }

    #[test]
    fn allowlist_repos_are_paired_ssh_and_https() {
        // Each entry should have a pair: if `github.com:foo/bar`
        // exists, so must `github.com/foo/bar`.
        for entry in INTERNAL_MODEL_REPOS {
            if let Some(rest) = entry.strip_prefix("github.com:") {
                let https = format!("github.com/{rest}");
                assert!(
                    INTERNAL_MODEL_REPOS.contains(&https.as_str()),
                    "ssh entry `{entry}` missing https counterpart `{https}`"
                );
            }
        }
    }

    #[test]
    fn classify_ssh_internal_matches() {
        let class = classify_remote_url(Some("git@github.com:anthropics/apps.git"));
        assert_eq!(class, RepoClass::Internal);
    }

    #[test]
    fn classify_https_internal_matches() {
        let class = classify_remote_url(Some(
            "https://github.com/anthropics/claude-cli-internal.git",
        ));
        assert_eq!(class, RepoClass::Internal);
    }

    #[test]
    fn classify_external_with_remote() {
        let class = classify_remote_url(Some("https://github.com/someorg/random-repo.git"));
        assert_eq!(class, RepoClass::External);
    }

    #[test]
    fn classify_no_remote() {
        assert_eq!(classify_remote_url(None), RepoClass::None);
        // Empty-but-Some is `None` — TS checks `if (!remoteUrl) {
        // return 'none' }` where empty string is falsy. Both
        // `None` and `Some("")` mean "no remote configured" so
        // they resolve to the same class. Prior to the codex CR
        // this was `External`; fixed for byte-exact TS parity.
        assert_eq!(classify_remote_url(Some("")), RepoClass::None);
    }

    #[test]
    fn classify_matches_substring_not_exact() {
        // TS uses remoteUrl.includes(repo). Any URL that contains
        // the allowlist entry as a substring is Internal.
        let weird = Some("git@myhost.git-proxy.internal:github.com:anthropics/apps/mirror.git");
        assert_eq!(classify_remote_url(weird), RepoClass::Internal);
    }

    #[test]
    fn undercover_non_ant_is_false() {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::remove_var("USER_TYPE");
        std::env::set_var("CLAUDE_CODE_UNDERCOVER", "1");
        assert!(!is_undercover());
        std::env::remove_var("CLAUDE_CODE_UNDERCOVER");
    }

    #[test]
    fn undercover_ant_force_on_via_env() {
        let _g = ENV_LOCK.lock().unwrap();
        reset_repo_class_cache();
        std::env::set_var("USER_TYPE", "ant");
        std::env::set_var("CLAUDE_CODE_UNDERCOVER", "true");
        // Prime to Internal — would normally turn off undercover,
        // but the env var forces it on.
        set_repo_class_for_test_or_prime(RepoClass::Internal);
        assert!(is_undercover());
        std::env::remove_var("CLAUDE_CODE_UNDERCOVER");
        std::env::remove_var("USER_TYPE");
        reset_repo_class_cache();
    }

    #[test]
    fn undercover_ant_auto_internal_off() {
        let _g = ENV_LOCK.lock().unwrap();
        reset_repo_class_cache();
        std::env::set_var("USER_TYPE", "ant");
        std::env::remove_var("CLAUDE_CODE_UNDERCOVER");
        set_repo_class_for_test_or_prime(RepoClass::Internal);
        assert!(!is_undercover());
        std::env::remove_var("USER_TYPE");
        reset_repo_class_cache();
    }

    #[test]
    fn undercover_ant_auto_external_on() {
        let _g = ENV_LOCK.lock().unwrap();
        reset_repo_class_cache();
        std::env::set_var("USER_TYPE", "ant");
        std::env::remove_var("CLAUDE_CODE_UNDERCOVER");
        set_repo_class_for_test_or_prime(RepoClass::External);
        assert!(is_undercover());
        std::env::remove_var("USER_TYPE");
        reset_repo_class_cache();
    }

    #[test]
    fn undercover_ant_auto_none_on() {
        let _g = ENV_LOCK.lock().unwrap();
        reset_repo_class_cache();
        std::env::set_var("USER_TYPE", "ant");
        std::env::remove_var("CLAUDE_CODE_UNDERCOVER");
        set_repo_class_for_test_or_prime(RepoClass::None);
        assert!(is_undercover());
        std::env::remove_var("USER_TYPE");
        reset_repo_class_cache();
    }

    #[test]
    fn undercover_ant_unprimed_on_conservative() {
        let _g = ENV_LOCK.lock().unwrap();
        reset_repo_class_cache();
        std::env::set_var("USER_TYPE", "ant");
        std::env::remove_var("CLAUDE_CODE_UNDERCOVER");
        // Cache intentionally NOT primed.
        assert!(is_undercover());
        std::env::remove_var("USER_TYPE");
    }

    #[test]
    fn is_internal_repo_cached_false_when_unprimed() {
        let _g = ENV_LOCK.lock().unwrap();
        reset_repo_class_cache();
        assert!(!is_internal_repo_cached());
    }

    #[test]
    fn is_internal_repo_cached_true_after_internal_prime() {
        let _g = ENV_LOCK.lock().unwrap();
        reset_repo_class_cache();
        set_repo_class_for_test_or_prime(RepoClass::Internal);
        assert!(is_internal_repo_cached());
        reset_repo_class_cache();
    }

    #[test]
    fn get_undercover_instructions_empty_for_non_ant() {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::remove_var("USER_TYPE");
        assert_eq!(get_undercover_instructions(), "");
    }

    #[test]
    fn get_undercover_instructions_nonempty_for_ant() {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::set_var("USER_TYPE", "ant");
        let s = get_undercover_instructions();
        assert!(s.contains("UNDERCOVER MODE"));
        assert!(s.contains("Capybara"));
        assert!(s.contains("Co-Authored-By"));
        std::env::remove_var("USER_TYPE");
    }

    #[test]
    fn set_repo_class_idempotent() {
        let _g = ENV_LOCK.lock().unwrap();
        reset_repo_class_cache();
        set_repo_class_for_test_or_prime(RepoClass::External);
        set_repo_class_for_test_or_prime(RepoClass::Internal); // ignored
        assert_eq!(get_repo_class_cached(), Some(RepoClass::External));
        reset_repo_class_cache();
    }
}
