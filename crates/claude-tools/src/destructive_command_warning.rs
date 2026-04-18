//! Destructive-command detection for BashTool.
//!
//! Port of `src/tools/BashTool/destructiveCommandWarning.ts`. Purely
//! informational — intended to surface a warning in the permission dialog
//! so the user sees "Note: may discard uncommitted changes" before they
//! approve. Does NOT affect auto-approval or permission logic.

use regex_lite::Regex;
use std::sync::OnceLock;

struct Pattern {
    re: Regex,
    warning: &'static str,
}

fn patterns() -> &'static [Pattern] {
    static CELL: OnceLock<Vec<Pattern>> = OnceLock::new();
    CELL.get_or_init(|| {
        // regex-lite does not support look-around / back-references; the TS
        // patterns stay within its feature set (character classes, groups,
        // alternation, anchors).
        let raw: &[(&str, &str)] = &[
            // Git — data loss / hard to reverse
            (r"\bgit\s+reset\s+--hard\b", "Note: may discard uncommitted changes"),
            (
                r"\bgit\s+push\b[^;&|\n]*[ \t](--force|--force-with-lease|-f)\b",
                "Note: may overwrite remote history",
            ),
            // clean -f (without --dry-run / -n)
            (
                r"\bgit\s+clean\b[^;&|\n]*-[a-zA-Z]*f",
                "Note: may permanently delete untracked files",
            ),
            (
                r"\bgit\s+checkout\s+(--\s+)?\.[ \t]*($|[;&|\n])",
                "Note: may discard all working tree changes",
            ),
            (
                r"\bgit\s+restore\s+(--\s+)?\.[ \t]*($|[;&|\n])",
                "Note: may discard all working tree changes",
            ),
            (
                r"\bgit\s+stash[ \t]+(drop|clear)\b",
                "Note: may permanently remove stashed changes",
            ),
            (
                r"\bgit\s+branch\s+(-D[ \t]|--delete\s+--force|--force\s+--delete)\b",
                "Note: may force-delete a branch",
            ),
            // Git — safety bypass
            (
                r"\bgit\s+(commit|push|merge)\b[^;&|\n]*--no-verify\b",
                "Note: may skip safety hooks",
            ),
            (
                r"\bgit\s+commit\b[^;&|\n]*--amend\b",
                "Note: may rewrite the last commit",
            ),
            // File deletion
            (
                r"(^|[;&|\n]\s*)rm\s+-[a-zA-Z]*[rR][a-zA-Z]*f|(^|[;&|\n]\s*)rm\s+-[a-zA-Z]*f[a-zA-Z]*[rR]",
                "Note: may recursively force-remove files",
            ),
            (
                r"(^|[;&|\n]\s*)rm\s+-[a-zA-Z]*[rR]",
                "Note: may recursively remove files",
            ),
            (
                r"(^|[;&|\n]\s*)rm\s+-[a-zA-Z]*f",
                "Note: may force-remove files",
            ),
            // Database (case-insensitive)
            (
                r"(?i)\b(DROP|TRUNCATE)\s+(TABLE|DATABASE|SCHEMA)\b",
                "Note: may drop or truncate database objects",
            ),
            (
                r#"(?i)\bDELETE\s+FROM\s+\w+[ \t]*(;|"|'|\n|$)"#,
                "Note: may delete all rows from a database table",
            ),
            // Infrastructure
            (r"\bkubectl\s+delete\b", "Note: may delete Kubernetes resources"),
            (r"\bterraform\s+destroy\b", "Note: may destroy Terraform infrastructure"),
        ];
        raw.iter()
            .map(|(re, w)| Pattern {
                re: Regex::new(re).expect("destructive pattern compiles"),
                warning: w,
            })
            .collect()
    })
}

/// Return the first matching destructive-command warning, or `None` if
/// the command does not match any known pattern. Deliberately returns
/// only one warning — further checks run at execution time.
pub fn get_destructive_command_warning(command: &str) -> Option<&'static str> {
    for p in patterns() {
        if p.re.is_match(command) {
            // Extra filter for `git clean`: if --dry-run / -n appears in the
            // same command, suppress the warning. regex-lite can't negate
            // inside the pattern, so check here.
            if p.warning.contains("delete untracked files")
                && (command.contains("--dry-run") || matches_clean_dry_run(command))
            {
                continue;
            }
            return Some(p.warning);
        }
    }
    None
}

fn matches_clean_dry_run(command: &str) -> bool {
    // `-n` short flag for dry-run (must be its own switch, not part of -nf)
    let re = Regex::new(r"\bgit\s+clean\b[^;&|\n]*\s-[a-zA-Z]*n").unwrap();
    re.is_match(command)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_git_reset_hard() {
        assert_eq!(
            get_destructive_command_warning("git reset --hard HEAD"),
            Some("Note: may discard uncommitted changes"),
        );
    }

    #[test]
    fn detects_force_push() {
        assert_eq!(
            get_destructive_command_warning("git push origin main --force"),
            Some("Note: may overwrite remote history"),
        );
        assert_eq!(
            get_destructive_command_warning("git push -f"),
            Some("Note: may overwrite remote history"),
        );
    }

    #[test]
    fn detects_rm_rf() {
        assert_eq!(
            get_destructive_command_warning("rm -rf node_modules"),
            Some("Note: may recursively force-remove files"),
        );
    }

    #[test]
    fn detects_drop_table() {
        assert_eq!(
            get_destructive_command_warning("DROP TABLE users;"),
            Some("Note: may drop or truncate database objects"),
        );
    }

    #[test]
    fn detects_kubectl_delete() {
        assert_eq!(
            get_destructive_command_warning("kubectl delete pod foo"),
            Some("Note: may delete Kubernetes resources"),
        );
    }

    #[test]
    fn ignores_safe_commands() {
        assert!(get_destructive_command_warning("ls -la").is_none());
        assert!(get_destructive_command_warning("git status").is_none());
        assert!(get_destructive_command_warning("echo hello").is_none());
    }

    #[test]
    fn dry_run_git_clean_suppresses_warning() {
        assert!(get_destructive_command_warning("git clean -f --dry-run").is_none());
        assert!(get_destructive_command_warning("git clean -nf").is_none());
    }
}
