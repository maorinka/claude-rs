//! BashTool read-only command classifier.
//!
//! The TS `readOnlyValidation.ts` + `utils/shell/readOnlyCommandValidation.ts`
//! together total ~3,900 LOC of per-flag allowlists (xargs, fd, ripgrep,
//! git, gh, pyright, docker, …). Porting that depth faithfully is its own
//! project; this module ships the coarser classifier the sandbox heuristic
//! and auto-approval path actually depend on: "is the first token in our
//! known-safe list, and does it lack obvious output redirection?".
//!
//! Callers that need flag-level scrutiny (e.g. refusing `xargs -I ... rm`)
//! MUST layer additional checks on top — this function intentionally does
//! not inspect subcommands or shell constructs.

use std::collections::HashSet;
use std::sync::OnceLock;

/// Result of classifying a bash command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Classification {
    /// Command is known to only read state.
    ReadOnly,
    /// Command is known to mutate state.
    Mutating,
    /// Unclassified — sandbox heuristic should fall through to conservative
    /// handling. Covers any unknown binary and any subcommand not in our
    /// small per-tool allowlist.
    Unknown,
}

/// Core set of binaries that are read-only by default regardless of args.
/// Derived from TS EXTERNAL_READONLY_COMMANDS plus trivially-safe POSIX
/// core utils. Conservative: anything that writes files, executes code,
/// or makes outbound network calls is omitted.
fn readonly_binaries() -> &'static HashSet<&'static str> {
    static CELL: OnceLock<HashSet<&'static str>> = OnceLock::new();
    CELL.get_or_init(|| {
        [
            // Listing / reading
            "ls",
            "cat",
            "head",
            "tail",
            "wc",
            "stat",
            "file",
            "type",
            "which",
            "whereis",
            "readlink",
            "realpath",
            "pwd",
            // Search
            "find",
            "locate",
            "grep",
            "egrep",
            "fgrep",
            "rg",
            "ripgrep",
            "ag",
            "fd",
            "fdfind",
            // Diff / comparison
            "diff",
            "cmp",
            // System info
            "uname",
            "hostname",
            "id",
            "whoami",
            "date",
            "uptime",
            "env",
            "echo",
            "printf",
            "true",
            "false",
            "yes",
            // Printing / paging
            "less",
            "more",
            // Text transforms that don't write files
            "awk",
            "sort",
            "uniq",
            "tr",
            "cut",
            "paste",
            "column",
            "nl",
            "tac",
            "rev",
            "fold",
            "basename",
            "dirname",
            // Checksums
            "md5sum",
            "sha1sum",
            "sha256sum",
            "sha512sum",
            "cksum",
            // Tree / metadata
            "tree",
            "du",
            "df",
            // Network diagnostics (read-only)
            "ping",
            "traceroute",
            "dig",
            "nslookup",
            "host",
            // Language docs
            "man",
        ]
        .into_iter()
        .collect()
    })
}

/// Binaries that obviously mutate state regardless of subcommand.
fn mutating_binaries() -> &'static HashSet<&'static str> {
    static CELL: OnceLock<HashSet<&'static str>> = OnceLock::new();
    CELL.get_or_init(|| {
        [
            "rm", "rmdir", "mv", "cp", "dd", "shred", "install", "touch", "mkdir", "ln", "chmod",
            "chown", "chgrp", "sed", // in-place via -i; treat the whole binary as mutating
            "curl", "wget", "ssh", "scp", "rsync", "npm", "yarn", "pnpm", "pip", "cargo",
            "go",
            // NB: `git`, `gh`, `docker` are NOT here — their first subcommand
            // decides via subcommand_allowlist().
        ]
        .into_iter()
        .collect()
    })
}

/// Per-binary read-only subcommand allowlists. Ported from TS
/// GIT_READ_ONLY_COMMANDS / GH_READ_ONLY_COMMANDS / DOCKER_READ_ONLY_COMMANDS.
fn subcommand_allowlist(binary: &str) -> Option<&'static HashSet<&'static str>> {
    static GIT: OnceLock<HashSet<&'static str>> = OnceLock::new();
    static GH: OnceLock<HashSet<&'static str>> = OnceLock::new();
    static DOCKER: OnceLock<HashSet<&'static str>> = OnceLock::new();
    match binary {
        "git" => Some(GIT.get_or_init(|| {
            [
                "status",
                "log",
                "show",
                "diff",
                "branch",
                "tag",
                "blame",
                "config",
                "describe",
                "reflog",
                "shortlog",
                "worktree",
                "rev-parse",
                "ls-files",
                "ls-tree",
                "cat-file",
                "grep",
                "name-rev",
                "help",
                "version",
                "remote",
                "stash",
                "for-each-ref",
                "check-ignore",
                "whatchanged",
            ]
            .into_iter()
            .collect()
        })),
        "gh" => Some(GH.get_or_init(|| {
            [
                "auth", "help", "version", "issue", "pr", "repo", "run", "workflow", "release",
                "status", "label", "api", "search", "gist", "variable",
            ]
            .into_iter()
            .collect()
        })),
        "docker" => Some(DOCKER.get_or_init(|| {
            [
                "ps", "images", "logs", "inspect", "history", "version", "info", "diff", "top",
                "stats", "events", "port", "search", "volume", "network",
            ]
            .into_iter()
            .collect()
        })),
        _ => None,
    }
}

/// Tokenise a shell command loosely. Does NOT fully implement shell
/// quoting; multi-word quoted strings collapse to single tokens only when
/// wrapped in matching quotes. Good enough for the classifier's purposes.
fn loose_tokens(cmd: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_single = false;
    let mut in_double = false;
    for c in cmd.chars() {
        match c {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            c if c.is_whitespace() && !in_single && !in_double => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Shell constructs that force us to bail out — anything involving
/// backticks, `$(...)`, redirections to files, pipes to non-readonly
/// binaries, or chained commands is classified Unknown. Callers can treat
/// Unknown as "ask the user first".
fn has_dangerous_construct(cmd: &str) -> bool {
    // Output redirection of any kind writes state.
    if cmd.contains(">>") || cmd.contains(" > ") || cmd.ends_with(">") {
        return true;
    }
    // Subshell / backtick substitution — the inner command is arbitrary.
    if cmd.contains("`") || cmd.contains("$(") {
        return true;
    }
    false
}

/// Classify a bash command.
pub fn classify_command(cmd: &str) -> Classification {
    let cmd = cmd.trim();
    if cmd.is_empty() {
        return Classification::Unknown;
    }
    if has_dangerous_construct(cmd) {
        return Classification::Unknown;
    }

    // Chained commands (`;` / `&&` / `||` / `|`) — classify each segment and
    // return the weakest of {ReadOnly, Mutating, Unknown}. If any segment is
    // Mutating, whole command is Mutating; if any is Unknown, Unknown.
    if cmd.contains("&&") || cmd.contains("||") || cmd.contains(';') || cmd.contains('|') {
        let segments = split_pipeline(cmd);
        let mut saw_mutating = false;
        let mut saw_unknown = false;
        for seg in segments {
            match classify_single(&seg) {
                Classification::Mutating => saw_mutating = true,
                Classification::Unknown => saw_unknown = true,
                Classification::ReadOnly => {}
            }
        }
        return if saw_mutating {
            Classification::Mutating
        } else if saw_unknown {
            Classification::Unknown
        } else {
            Classification::ReadOnly
        };
    }

    classify_single(cmd)
}

fn split_pipeline(cmd: &str) -> Vec<String> {
    // Cheap splitter that respects quotes; treats `|` / `||` / `&&` / `;`
    // as segment boundaries. We don't distinguish `||` vs `|` here since
    // we only need the segments.
    let mut segments = Vec::new();
    let mut cur = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let bytes = cmd.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        match c {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '|' | '&' | ';' if !in_single && !in_double => {
                // Consume doubled operators (||, &&) as one separator.
                if (c == '|' || c == '&') && i + 1 < bytes.len() && bytes[i + 1] as char == c {
                    i += 1;
                }
                if !cur.trim().is_empty() {
                    segments.push(std::mem::take(&mut cur).trim().to_string());
                }
                i += 1;
                continue;
            }
            _ => {}
        }
        cur.push(c);
        i += 1;
    }
    if !cur.trim().is_empty() {
        segments.push(cur.trim().to_string());
    }
    segments
}

fn classify_single(cmd: &str) -> Classification {
    let tokens = loose_tokens(cmd);
    let Some(first) = tokens.first() else {
        return Classification::Unknown;
    };
    let binary = strip_env_assignments(&tokens);
    let Some(binary) = binary else {
        return Classification::Unknown;
    };

    // Quick checks first: known mutating, known read-only, …
    if mutating_binaries().contains(binary) {
        return Classification::Mutating;
    }
    if readonly_binaries().contains(binary) {
        return Classification::ReadOnly;
    }

    if binary == "git" {
        return classify_git_command(&tokens, binary);
    }

    // Per-binary subcommand allowlists (gh, docker).
    if let Some(allow) = subcommand_allowlist(binary) {
        let sub = tokens
            .iter()
            .skip_while(|t| t.as_str() != binary)
            .skip(1)
            .find(|t| !t.starts_with('-'))
            .map(String::as_str);
        return match sub {
            Some(s) if allow.contains(s) => Classification::ReadOnly,
            Some(_) => Classification::Mutating,
            None => Classification::ReadOnly, // bare `gh`/`docker` prints help
        };
    }

    let _ = first;
    Classification::Unknown
}

fn classify_git_command(tokens: &[String], binary: &str) -> Classification {
    let Some(git_index) = tokens.iter().position(|t| t == binary) else {
        return Classification::Unknown;
    };
    let args = &tokens[git_index + 1..];
    let Some(sub) = args
        .iter()
        .find(|t| !t.starts_with('-'))
        .map(String::as_str)
    else {
        return Classification::ReadOnly;
    };
    let sub_index = args.iter().position(|t| t == sub).unwrap_or(0);
    let rest = &args[sub_index + 1..];

    match sub {
        "status" | "log" | "show" | "diff" | "branch" | "tag" | "blame" | "describe" | "reflog"
        | "shortlog" | "rev-parse" | "ls-files" | "ls-tree" | "cat-file" | "grep" | "name-rev"
        | "help" | "version" | "for-each-ref" | "check-ignore" | "whatchanged" => {
            Classification::ReadOnly
        }
        "stash" if rest.first().is_some_and(|arg| arg == "list") => Classification::ReadOnly,
        "worktree" if rest.first().is_some_and(|arg| arg == "list") => Classification::ReadOnly,
        "config" if rest.first().is_some_and(|arg| arg == "--get") => Classification::ReadOnly,
        "remote" if git_remote_is_read_only(rest) => Classification::ReadOnly,
        _ => Classification::Mutating,
    }
}

fn git_remote_is_read_only(args: &[String]) -> bool {
    if args.is_empty() {
        return true;
    }
    if args.iter().all(|arg| arg == "-v" || arg == "--verbose") {
        return true;
    }
    if args.first().is_some_and(|arg| arg == "show") {
        let positional = args
            .iter()
            .skip(1)
            .filter(|arg| arg.as_str() != "-n")
            .collect::<Vec<_>>();
        return positional.len() == 1
            && positional[0]
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    }
    false
}

/// If the command starts with `VAR=value VAR2=value2 <binary>`, return the
/// binary token. Otherwise return the first token.
fn strip_env_assignments(tokens: &[String]) -> Option<&str> {
    for t in tokens {
        // An env assignment is `NAME=...` where NAME is all alnum/_ starting
        // with a non-digit.
        let bytes = t.as_bytes();
        let eq = bytes.iter().position(|&b| b == b'=');
        let is_env = eq.is_some_and(|i| {
            i > 0 && bytes[0].is_ascii_alphabetic()
                || bytes[0] == b'_'
                    && bytes[..i]
                        .iter()
                        .all(|&b| b.is_ascii_alphanumeric() || b == b'_')
        });
        if !is_env {
            return Some(t.as_str());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ls_is_readonly() {
        assert_eq!(classify_command("ls -la /tmp"), Classification::ReadOnly);
    }

    #[test]
    fn rm_is_mutating() {
        assert_eq!(classify_command("rm file.txt"), Classification::Mutating);
    }

    #[test]
    fn unknown_binary_is_unknown() {
        assert_eq!(classify_command("myweirdbin arg"), Classification::Unknown);
    }

    #[test]
    fn git_status_is_readonly() {
        assert_eq!(classify_command("git status"), Classification::ReadOnly);
        assert_eq!(
            classify_command("git log --oneline -5"),
            Classification::ReadOnly
        );
        assert_eq!(
            classify_command("git diff --cached"),
            Classification::ReadOnly
        );
    }

    #[test]
    fn git_rev_list_requires_permission_like_ts() {
        assert_eq!(
            classify_command("git rev-list --left-right --count main...HEAD"),
            Classification::Mutating
        );
        assert_eq!(
            classify_command("git status && git rev-list --left-right --count main...HEAD"),
            Classification::Mutating
        );
    }

    #[test]
    fn git_commit_is_mutating() {
        assert_eq!(
            classify_command("git commit -m hello"),
            Classification::Mutating
        );
        assert_eq!(classify_command("git push"), Classification::Mutating);
    }

    #[test]
    fn git_multiword_readonly_forms_match_ts() {
        assert_eq!(
            classify_command("git worktree list"),
            Classification::ReadOnly
        );
        assert_eq!(
            classify_command("git stash list --oneline"),
            Classification::ReadOnly
        );
        assert_eq!(
            classify_command("git config --get user.name"),
            Classification::ReadOnly
        );
        assert_eq!(classify_command("git remote -v"), Classification::ReadOnly);
        assert_eq!(
            classify_command("git remote show origin"),
            Classification::ReadOnly
        );
    }

    #[test]
    fn git_multiword_mutating_forms_require_permission_like_ts() {
        assert_eq!(
            classify_command("git worktree add ../tmp HEAD"),
            Classification::Mutating
        );
        assert_eq!(classify_command("git stash pop"), Classification::Mutating);
        assert_eq!(
            classify_command("git config user.name test"),
            Classification::Mutating
        );
        assert_eq!(
            classify_command("git remote add origin https://example.com/repo.git"),
            Classification::Mutating
        );
    }

    #[test]
    fn output_redirect_is_unknown() {
        assert_eq!(classify_command("ls > out.txt"), Classification::Unknown);
        assert_eq!(classify_command("echo hi >> log"), Classification::Unknown);
    }

    #[test]
    fn command_substitution_is_unknown() {
        assert_eq!(
            classify_command("echo $(rm -rf /)"),
            Classification::Unknown
        );
        assert_eq!(classify_command("echo `date`"), Classification::Unknown);
    }

    #[test]
    fn pipe_of_readonly_is_readonly() {
        assert_eq!(
            classify_command("ls -la | grep foo | head"),
            Classification::ReadOnly
        );
    }

    #[test]
    fn pipe_into_mutating_is_mutating() {
        assert_eq!(
            classify_command("cat file.txt | rm -rf /tmp"),
            Classification::Mutating
        );
    }

    #[test]
    fn chained_mixed_takes_strongest_risk() {
        // ReadOnly + Mutating → Mutating
        assert_eq!(
            classify_command("ls && rm -rf tmp"),
            Classification::Mutating
        );
        // ReadOnly + Unknown → Unknown
        assert_eq!(
            classify_command("ls && myweirdbin"),
            Classification::Unknown
        );
    }

    #[test]
    fn env_assignment_prefix_handled() {
        assert_eq!(classify_command("FOO=bar ls -la"), Classification::ReadOnly);
    }

    #[test]
    fn docker_inspect_is_readonly_but_run_is_mutating() {
        assert_eq!(classify_command("docker ps"), Classification::ReadOnly);
        assert_eq!(classify_command("docker run img"), Classification::Mutating);
    }
}
