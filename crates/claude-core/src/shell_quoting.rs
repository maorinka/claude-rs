//! Shell-command quoting helpers with heredoc / multiline / stdin-
//! redirect awareness.
//!
//! Byte-for-byte port of `utils/bash/shellQuoting.ts` (128 LOC).
//! The module sits alongside the lower-level `shell_quote` parser
//! and adds higher-level policy: "given a user command, produce a
//! shell-safe wrapping that preserves heredocs and multiline
//! strings, and optionally appends `< /dev/null` to prevent
//! interactive prompts from blocking the subprocess."
//!
//! All helpers are pure string operations. Uses pre-compiled
//! `regex::Regex` instances via `once_cell::Lazy` to avoid
//! per-call compile cost.

use once_cell::sync::Lazy;
use regex::Regex;

use crate::shell_quote::quote;

/// Matches heredoc openers: `<<EOF`, `<<'EOF'`, `<<"EOF"`,
/// `<<-EOF`, `<<\EOF`. TS at `shellQuoting.ts:20` uses a
/// backreference (`\1`) to match the same quote character on
/// both sides of the terminator. Rust's `regex` crate is
/// backtrack-free and rejects backrefs, so the four quote
/// forms are enumerated explicitly. Functionally equivalent:
/// either `'EOF'` OR `"EOF"` OR bare `EOF` OR `\EOF` matches.
static HEREDOC_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"<<-?\s*(?:'(?:\w+)'|"(?:\w+)"|\\(?:\w+)|(?:\w+))"#).unwrap());

/// Bit-shift operator patterns that must NOT count as heredocs.
/// TS at `shellQuoting.ts:12-16`. Three distinct forms — each
/// arithmetic context where `<<` means left-shift.
static BIT_SHIFT_INT: Lazy<Regex> = Lazy::new(|| Regex::new(r#"\d\s*<<\s*\d"#).unwrap());
static BIT_SHIFT_BRACKETS: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"\[\[\s*\d+\s*<<\s*\d+\s*\]\]"#).unwrap());
static BIT_SHIFT_ARITH: Lazy<Regex> = Lazy::new(|| Regex::new(r#"\$\(\(.*<<.*\)\)"#).unwrap());

/// Multiline single-quoted string: `'...\n...'` with escape
/// support for `\'`. TS at `shellQuoting.ts:32`.
static SINGLE_QUOTE_MULTILINE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"'(?:[^'\\]|\\.)*\n(?:[^'\\]|\\.)*'"#).unwrap());
/// Multiline double-quoted string: `"...\n..."` with escape
/// support for `\"`. TS at `shellQuoting.ts:33`.
static DOUBLE_QUOTE_MULTILINE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#""(?:[^"\\]|\\.)*\n(?:[^"\\]|\\.)*""#).unwrap());

/// `< file` / `< /dev/null` detector. Excludes `<<` (heredoc)
/// and `<(` (process substitution). Must be preceded by
/// whitespace / separator / start-of-string. TS at
/// `shellQuoting.ts:85` uses a negative lookahead `(?![<(])`,
/// which the Rust regex crate doesn't support. Rewritten as
/// `<[^<(]` — the char immediately after `<` must not be `<`
/// or `(`. The TS `\s*\S+` tail (filename) is kept to enforce
/// "a `<` followed by an actual argument".
static STDIN_REDIRECT: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?:^|[\s;&|])<[^<(]\s*\S*"#).unwrap());

/// Windows CMD `>nul` pattern, case-insensitive. TS comment at
/// `shellQuoting.ts:124`: `>nul`, `> NUL`, `2>nul`, `&>nul`,
/// `>>nul`. Must NOT match `>null`, `>nullable`, `>nul.txt`.
/// The trailing lookahead keeps boundary safety.
///
/// Rust `regex` crate doesn't support lookahead, so we
/// match an optional "terminator" character explicitly and
/// handle the end-of-string case with `$`.
static NUL_REDIRECT_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?i)(\d?&?>+\s*)nul(\s|$|[|&;)\n])"#).unwrap());

/// Detect a heredoc opener in `command`. Returns `false` when
/// the `<<` is actually a bit-shift operator (three common
/// arithmetic forms). TS at `shellQuoting.ts:7-22`.
pub fn contains_heredoc(command: &str) -> bool {
    // Bit-shift short-circuit first — matches TS ordering.
    if BIT_SHIFT_INT.is_match(command)
        || BIT_SHIFT_BRACKETS.is_match(command)
        || BIT_SHIFT_ARITH.is_match(command)
    {
        return false;
    }
    HEREDOC_REGEX.is_match(command)
}

/// Detect a multiline string (single- or double-quoted) inside
/// `command`. Escaped quotes `\'` / `\"` don't terminate the
/// string. TS at `shellQuoting.ts:27-38`.
pub fn contains_multiline_string(command: &str) -> bool {
    SINGLE_QUOTE_MULTILINE.is_match(command) || DOUBLE_QUOTE_MULTILINE.is_match(command)
}

/// Does `command` already contain a stdin-redirect like
/// `< file` or `< /dev/null`? TS at `shellQuoting.ts:81-86`.
/// Excludes `<<` (heredoc) and `<(` (process substitution).
pub fn has_stdin_redirect(command: &str) -> bool {
    STDIN_REDIRECT.is_match(command)
}

/// Is it safe to append a stdin redirect to `command`? TS at
/// `shellQuoting.ts:93-106`.
///
/// Returns `false` when:
/// - the command contains a heredoc (stdin is already the
///   heredoc body), or
/// - the command already has its own stdin redirect.
pub fn should_add_stdin_redirect(command: &str) -> bool {
    if contains_heredoc(command) {
        return false;
    }
    if has_stdin_redirect(command) {
        return false;
    }
    true
}

/// Quote a shell command for safe eval. Matches TS
/// `quoteShellCommand` at `shellQuoting.ts:46-74`.
///
/// Behaviour:
/// - Commands with heredocs / multiline strings use a literal
///   single-quote wrap with `'` escaping, NOT the
///   `shell-quote` parser — TS discovered shell-quote's
///   aggressive `!` escaping corrupts those forms.
///   Heredocs suppress the stdin-redirect append regardless of
///   `add_stdin_redirect`.
/// - Plain commands go through `shell_quote::quote` with an
///   optional `< /dev/null` suffix.
pub fn quote_shell_command(command: &str, add_stdin_redirect: bool) -> String {
    if contains_heredoc(command) || contains_multiline_string(command) {
        // Manual single-quote wrap — escape only single quotes.
        let escaped = command.replace('\'', "'\"'\"'");
        let quoted = format!("'{}'", escaped);
        if contains_heredoc(command) {
            return quoted;
        }
        return if add_stdin_redirect {
            format!("{} < /dev/null", quoted)
        } else {
            quoted
        };
    }
    if add_stdin_redirect {
        quote([command, "<", "/dev/null"])
    } else {
        quote([command])
    }
}

/// `for|while|until|if|case|select` keyword at the start of a
/// "word" (preceded by a word-boundary, followed by
/// whitespace). Used by the pipe-command rearranger to bail
/// out of the shell-quote parse — those control structures
/// confuse the parser and cause pipe boundaries inside the
/// body to be misclassified. Matches TS `containsControlStructure`
/// at `utils/bash/bashPipeCommand.ts:247-249`.
static CONTROL_STRUCTURE_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"\b(for|while|until|if|case|select)\s"#).unwrap());

/// `\\+\n` — one or more backslashes at end-of-line followed
/// by a newline. Used by `join_continuation_lines` to collapse
/// bash line continuations. Matches TS
/// `joinContinuationLines` at `utils/bash/bashPipeCommand.ts:284`.
static CONTINUATION_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r#"\\+\n"#).unwrap());

/// Bash list-separators — operators that separate top-level
/// commands in a compound command. Used to detect "this looks
/// like a pure list of commands" during permission analysis.
/// Matches TS `COMMAND_LIST_SEPARATORS` at
/// `utils/bash/commands.ts:521-529`.
pub const COMMAND_LIST_SEPARATORS: &[&str] = &["&&", "||", ";", ";;", "|"];

/// Every control operator the shell-quote path recognises:
/// list separators plus the three stdout-redirect operators.
/// Matches TS `ALL_SUPPORTED_CONTROL_OPERATORS` at
/// `utils/bash/commands.ts:531-536`.
pub const ALL_SUPPORTED_CONTROL_OPERATORS: &[&str] = &["&&", "||", ";", ";;", "|", ">&", ">", ">>"];

/// Filter a list of tokens down to just the non-operator
/// command pieces. Matches TS `filterControlOperators` at
/// `utils/bash/commands.ts:251-257`.
///
/// Used by the permission analyzer to present the user with
/// only the actual subcommands inside a compound expression,
/// hiding the shell plumbing (`|`, `&&`, `>>`, etc.). The TS
/// version takes the string-form tokens emitted by a
/// shell-quote tokenise + rebuild pass; Rust keeps the same
/// stringly-typed contract so call sites can `&[&str]` or
/// `Vec<String>` interchangeably.
pub fn filter_control_operators<I, S>(parts: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    parts
        .into_iter()
        .filter_map(|p| {
            let s = p.as_ref();
            if ALL_SUPPORTED_CONTROL_OPERATORS.contains(&s) {
                None
            } else {
                Some(s.to_string())
            }
        })
        .collect()
}

/// Detect a "simple help invocation" — a command like
/// `tool --help` or `foo bar --help` where the ONLY flag is
/// `--help` and all non-flag tokens are plain ASCII
/// alphanumeric. Matches TS `isHelpCommand` at
/// `utils/bash/commands.ts:388-436`.
///
/// Used to skip permission prompts for help invocations (help
/// output is read-only and safe). The strict alphanumeric
/// check on non-flag tokens rejects paths, shell special
/// characters, and quoted arguments — those could smuggle
/// side effects through a shell injection.
///
/// Returns `false` for:
/// - commands not ending in `--help`,
/// - commands containing quotes (`'` or `"`),
/// - shell-parse failures,
/// - non-alphanumeric non-flag tokens (e.g. `./script --help`
///   has `.` which isn't alphanumeric),
/// - any flag other than `--help`.
pub fn is_help_command(command: &str) -> bool {
    use crate::shell_quote::{try_parse_shell_command, ParseEntry, ShellParseResult};

    let trimmed = command.trim();

    if !trimmed.ends_with("--help") {
        return false;
    }
    // Quote smuggling guard — TS at commands.ts:397.
    if trimmed.contains('"') || trimmed.contains('\'') {
        return false;
    }

    let tokens = match try_parse_shell_command(trimmed) {
        ShellParseResult::Ok(tokens) => tokens,
        ShellParseResult::Err(_) => return false,
    };

    let mut found_help = false;
    for entry in tokens {
        if let ParseEntry::Literal(token) = entry {
            if token.starts_with('-') {
                // A flag. The only allowed flag is `--help`.
                if token == "--help" {
                    found_help = true;
                } else {
                    return false;
                }
            } else if !token.chars().all(|c| c.is_ascii_alphanumeric()) {
                // Non-flag token with any non-alphanumeric char
                // — reject. Matches TS `alphanumericPattern` at
                // commands.ts:411.
                return false;
            }
        }
        // Operators are not `Literal`; TS `typeof token ===
        // 'string'` check at commands.ts:414 similarly skips
        // them. Commands that parse with operators and still
        // end in `--help` are edge cases — left conservative.
    }

    found_help
}

/// Detect a bash control-structure keyword in `command` — any of
/// `for|while|until|if|case|select` at a word boundary followed
/// by whitespace. Used as a bail-out signal before feeding a
/// command to the shell-quote parser; those constructs confuse
/// the parser into misidentifying pipe boundaries inside the
/// loop body. Matches TS `containsControlStructure`
/// at `utils/bash/bashPipeCommand.ts:247`.
pub fn contains_control_structure(command: &str) -> bool {
    CONTROL_STRUCTURE_REGEX.is_match(command)
}

/// Single-quote a string for use as an `eval` argument.
/// Embedded single quotes are escaped via `'"'"'` — close the
/// current single-quoted segment, emit a literal quote inside
/// a short double-quoted segment, then reopen the single-
/// quoted segment. Matches TS `singleQuoteForEval` at
/// `utils/bash/bashPipeCommand.ts:273-275`.
///
/// Preferred over `shell_quote::quote` for commands containing
/// `'` because `quote` switches to double-quote mode when the
/// input has single quotes and then escapes `!` to `\!`,
/// corrupting `jq` / `awk` filters like `.x != .y`.
pub fn single_quote_for_eval(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\"'\"'"))
}

/// Join bash line-continuation backslash-newlines into a
/// single line. Only collapses sequences with an ODD number of
/// backslashes before the newline — the last backslash escapes
/// the newline. Even backslashes pair up as literal backslash-
/// escapes and the newline stays a separator. Matches TS
/// `joinContinuationLines` at
/// `utils/bash/bashPipeCommand.ts:283-294`.
pub fn join_continuation_lines(command: &str) -> String {
    CONTINUATION_REGEX
        .replace_all(command, |caps: &regex::Captures| {
            let m = &caps[0];
            // `m` is like `\\n`, `\\\\\n`, etc. The `\n` is
            // always one byte; the backslashes fill the rest.
            let backslash_count = m.len() - 1;
            if backslash_count % 2 == 1 {
                // Odd → last backslash escapes newline. Emit
                // the pairs that paired up, drop the last
                // backslash and the newline.
                "\\".repeat(backslash_count - 1)
            } else {
                // Even → newline survives. Emit as-is.
                m.to_string()
            }
        })
        .into_owned()
}

/// Rewrite Windows CMD `>nul` redirects to POSIX `/dev/null`.
/// TS at `shellQuoting.ts:108-128` — guards against models
/// hallucinating Windows shell syntax inside Git Bash / WSL,
/// which would otherwise create a literal `nul` file (a
/// reserved device name on Windows that's hard to delete and
/// breaks git commands). See anthropics/claude-code#4928.
///
/// The Rust regex replaces `nul` with `/dev/null` while
/// preserving the captured terminator character (whitespace,
/// shell operator, or `\n`) so `ls 2>nul;more` rewrites to
/// `ls 2>/dev/null;more`.
pub fn rewrite_windows_null_redirect(command: &str) -> String {
    // Rust `regex` replace_all doesn't have TS's `$1` by name
    // but supports `${1}` placeholders. The second capture is
    // the terminator (always present — `\s|$|[|&;)\n]` —
    // empty string when `$` matched).
    NUL_REDIRECT_REGEX
        .replace_all(command, "${1}/dev/null${2}")
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── contains_heredoc ────────────────────────────────────────

    #[test]
    fn heredoc_matches_standard_forms() {
        assert!(contains_heredoc("cat <<EOF\nhi\nEOF"));
        assert!(contains_heredoc("cat <<'EOF'\nhi\nEOF"));
        assert!(contains_heredoc("cat <<\"EOF\"\nhi\nEOF"));
        assert!(contains_heredoc("cat <<-EOF\n\thi\nEOF"));
        assert!(contains_heredoc("cat <<\\EOF\nhi\nEOF"));
    }

    #[test]
    fn heredoc_rejects_bit_shifts() {
        // Classic left-shift, `1 << 3`.
        assert!(!contains_heredoc("echo $((1 << 3))"));
        // Double-bracket form.
        assert!(!contains_heredoc("if [[ 1 << 3 ]]; then true; fi"));
        // Arithmetic context.
        assert!(!contains_heredoc("x=$((n << 2))"));
    }

    #[test]
    fn heredoc_rejects_plain_commands() {
        assert!(!contains_heredoc("echo hello"));
        assert!(!contains_heredoc("ls -la"));
    }

    // ─── contains_multiline_string ───────────────────────────────

    #[test]
    fn multiline_single_quoted() {
        assert!(contains_multiline_string("echo 'line1\nline2'"));
        // Escaped single quote inside doesn't terminate.
        assert!(contains_multiline_string("echo 'a\\'b\nc'"));
    }

    #[test]
    fn multiline_double_quoted() {
        assert!(contains_multiline_string("echo \"line1\nline2\""));
    }

    #[test]
    fn multiline_rejects_single_line_quoted() {
        assert!(!contains_multiline_string("echo 'hello'"));
        assert!(!contains_multiline_string("echo \"hello\""));
    }

    // ─── has_stdin_redirect / should_add_stdin_redirect ──────────

    #[test]
    fn stdin_redirect_detection() {
        assert!(has_stdin_redirect("cat < file.txt"));
        assert!(has_stdin_redirect("cat </path/to/file"));
        assert!(has_stdin_redirect("tee < /dev/null"));
        assert!(has_stdin_redirect("a; grep foo < bar"));

        // Heredoc is NOT a stdin redirect.
        assert!(!has_stdin_redirect("cat <<EOF\nhi\nEOF"));
        // Process substitution is NOT a stdin redirect.
        assert!(!has_stdin_redirect("diff <(ls) <(ls -a)"));
        // Bare less-than operator shouldn't falsely match.
        assert!(!has_stdin_redirect("echo hello"));
    }

    #[test]
    fn should_add_stdin_redirect_honours_existing_state() {
        assert!(should_add_stdin_redirect("ls -la"));
        // Heredoc command already has its own stdin.
        assert!(!should_add_stdin_redirect("cat <<EOF\nhi\nEOF"));
        // Existing stdin redirect → no-op.
        assert!(!should_add_stdin_redirect("cat < file.txt"));
    }

    // ─── quote_shell_command ─────────────────────────────────────

    #[test]
    fn quote_plain_command_with_stdin() {
        // Regular command + add_stdin_redirect=true → appends
        // `< /dev/null` through shell-quote.
        let out = quote_shell_command("ls -la", true);
        assert!(out.contains("/dev/null"));
        assert!(out.contains("ls -la"));
    }

    #[test]
    fn quote_plain_command_without_stdin() {
        let out = quote_shell_command("ls -la", false);
        assert!(!out.contains("/dev/null"));
    }

    #[test]
    fn quote_heredoc_suppresses_stdin_regardless_of_flag() {
        // Heredoc commands get wrapped manually AND skip the
        // stdin-redirect append even when the caller passes
        // true.
        let cmd = "cat <<EOF\nhi\nEOF";
        let with = quote_shell_command(cmd, true);
        let without = quote_shell_command(cmd, false);
        assert_eq!(with, without);
        assert!(!with.contains("/dev/null"));
        // Must start + end with single quote.
        assert!(with.starts_with('\''));
        assert!(with.ends_with('\''));
    }

    #[test]
    fn quote_multiline_without_heredoc_appends_stdin_when_requested() {
        let cmd = "echo 'line1\nline2'";
        let with = quote_shell_command(cmd, true);
        assert!(with.contains("< /dev/null"));
        let without = quote_shell_command(cmd, false);
        assert!(!without.contains("/dev/null"));
    }

    #[test]
    fn quote_escapes_single_quotes_in_heredoc_command() {
        // Inner single quote → `'"'"'` replacement so the
        // outer single-quote wrap isn't broken.
        let cmd = "cat <<EOF\n'inner'\nEOF";
        let out = quote_shell_command(cmd, false);
        assert!(out.contains("'\"'\"'inner'\"'\"'"));
    }

    // ─── rewrite_windows_null_redirect ───────────────────────────

    #[test]
    fn rewrite_nul_basic_forms() {
        assert_eq!(rewrite_windows_null_redirect("ls >nul"), "ls >/dev/null");
        assert_eq!(rewrite_windows_null_redirect("ls 2>nul"), "ls 2>/dev/null");
        assert_eq!(rewrite_windows_null_redirect("ls &>nul"), "ls &>/dev/null");
        assert_eq!(rewrite_windows_null_redirect("ls >>nul"), "ls >>/dev/null");
    }

    #[test]
    fn rewrite_nul_case_insensitive() {
        assert_eq!(rewrite_windows_null_redirect("ls > NUL"), "ls > /dev/null");
        assert_eq!(rewrite_windows_null_redirect("ls >Nul"), "ls >/dev/null");
    }

    #[test]
    fn rewrite_nul_guards_against_false_positives() {
        // `>null`, `>nullable`, `>nul.txt` must stay untouched.
        assert_eq!(rewrite_windows_null_redirect("echo >null"), "echo >null");
        assert_eq!(rewrite_windows_null_redirect("ls >nul.txt"), "ls >nul.txt");
        assert_eq!(rewrite_windows_null_redirect("cat nul.txt"), "cat nul.txt");
    }

    #[test]
    fn rewrite_nul_preserves_terminator() {
        // Terminator (`;`, `|`, whitespace) stays intact.
        assert_eq!(
            rewrite_windows_null_redirect("ls >nul;more"),
            "ls >/dev/null;more"
        );
        assert_eq!(
            rewrite_windows_null_redirect("ls >nul | grep x"),
            "ls >/dev/null | grep x"
        );
    }

    #[test]
    fn rewrite_nul_end_of_string() {
        // End-of-string must also terminate the match.
        assert_eq!(rewrite_windows_null_redirect("ls >nul"), "ls >/dev/null");
    }

    // ─── contains_control_structure ──────────────────────────────

    #[test]
    fn control_structure_matches_bash_keywords() {
        assert!(contains_control_structure(
            "for i in 1 2 3; do echo $i; done"
        ));
        assert!(contains_control_structure(
            "while read line; do echo hi; done"
        ));
        assert!(contains_control_structure(
            "until [[ $x -eq 0 ]]; do : ; done"
        ));
        assert!(contains_control_structure("if [[ -f file ]]; then ls; fi"));
        assert!(contains_control_structure("case $x in a) echo a ;; esac"));
        assert!(contains_control_structure("select opt in a b; do : ; done"));
    }

    #[test]
    fn control_structure_rejects_plain_commands() {
        assert!(!contains_control_structure("echo hello"));
        assert!(!contains_control_structure("ls -la | grep foo"));
        // Keyword embedded in a word doesn't count — word-
        // boundary + whitespace guard.
        assert!(!contains_control_structure("./forever"));
        assert!(!contains_control_structure("cat /etc/forward-list"));
    }

    // ─── single_quote_for_eval ───────────────────────────────────

    #[test]
    fn single_quote_for_eval_wraps_plain_string() {
        assert_eq!(single_quote_for_eval("echo hi"), "'echo hi'");
    }

    #[test]
    fn single_quote_for_eval_escapes_inner_quotes() {
        // `jq 'select(.x != .y)'` inside a single-quoted wrap
        // becomes `'jq '"'"'select(.x != .y)'"'"''`.
        let out = single_quote_for_eval("jq 'select(.x != .y)'");
        assert_eq!(out, "'jq '\"'\"'select(.x != .y)'\"'\"''");
    }

    #[test]
    fn single_quote_for_eval_preserves_bangs() {
        // The whole reason this exists: avoid `!` → `\!`
        // corruption that `shell_quote::quote` introduces when
        // switching to double-quote mode.
        let out = single_quote_for_eval(".x != .y");
        assert_eq!(out, "'.x != .y'");
        assert!(!out.contains("\\!"));
    }

    // ─── join_continuation_lines ─────────────────────────────────

    #[test]
    fn continuation_odd_backslashes_join() {
        // `\<newline>` → collapse, newline swallowed.
        let input = "echo hi \\\nthere";
        assert_eq!(join_continuation_lines(input), "echo hi there");
    }

    #[test]
    fn continuation_even_backslashes_preserve_newline() {
        // `\\<newline>` → the two backslashes pair up as an
        // escaped backslash, the newline stays a separator.
        let input = "echo hi \\\\\nthere";
        assert_eq!(join_continuation_lines(input), input);
    }

    #[test]
    fn continuation_three_backslashes_join_with_one_remaining() {
        // Three backslashes: one pair + one that escapes the
        // newline. Result: one literal backslash, no newline.
        let input = "echo hi \\\\\\\nthere";
        assert_eq!(join_continuation_lines(input), "echo hi \\\\there");
    }

    #[test]
    fn continuation_no_backslash_passthrough() {
        assert_eq!(join_continuation_lines("echo\nhello"), "echo\nhello");
    }

    #[test]
    fn continuation_multiple_lines() {
        let input = "a \\\nb \\\nc";
        // Two continuations → all three pieces join on one line.
        assert_eq!(join_continuation_lines(input), "a b c");
    }

    // ─── is_help_command ─────────────────────────────────────────

    #[test]
    fn help_command_matches_plain_help_invocations() {
        assert!(is_help_command("git --help"));
        assert!(is_help_command("cargo --help"));
        assert!(is_help_command("  git --help  ")); // trim
        assert!(is_help_command("git subcommand --help"));
        assert!(is_help_command("tool foo bar --help"));
    }

    #[test]
    fn help_command_rejects_without_help_suffix() {
        assert!(!is_help_command("git status"));
        assert!(!is_help_command("git --help status")); // --help not last
        assert!(!is_help_command(""));
    }

    #[test]
    fn help_command_rejects_quoted_commands() {
        // Quote-smuggling guard — a quoted `--help` could hide
        // a payload.
        assert!(!is_help_command("git \"--help\""));
        assert!(!is_help_command("git '--help'"));
        assert!(!is_help_command("echo 'x' --help"));
    }

    #[test]
    fn help_command_rejects_extra_flags() {
        // Only `--help` is allowed. Any other flag → reject.
        assert!(!is_help_command("git -v --help"));
        assert!(!is_help_command("tool --verbose --help"));
        assert!(!is_help_command("tool -h --help"));
    }

    #[test]
    fn help_command_rejects_non_alphanumeric_non_flag_tokens() {
        // Paths and special characters are rejected because they
        // could hide injection via tab-completion / path tricks.
        assert!(!is_help_command("./script --help"));
        assert!(!is_help_command("/usr/bin/tool --help"));
        assert!(!is_help_command("tool foo.bar --help"));
        assert!(!is_help_command("tool foo/bar --help"));
    }

    #[test]
    fn help_command_accepts_alphanumeric_subcommands() {
        // Subcommands / args with only letters & digits pass.
        assert!(is_help_command("git clone --help"));
        assert!(is_help_command("cargo test2 --help"));
        assert!(is_help_command("gh pr list --help"));
    }

    // ─── filter_control_operators ────────────────────────────────

    #[test]
    fn filter_removes_list_separators() {
        let input = vec!["ls", "|", "grep", "foo", "&&", "echo", "done"];
        let out = filter_control_operators(input);
        assert_eq!(out, vec!["ls", "grep", "foo", "echo", "done"]);
    }

    #[test]
    fn filter_removes_redirect_operators() {
        let input = vec!["ls", ">", "file.txt", ">>", "log", "2>&1"];
        let out = filter_control_operators(input);
        // `>`, `>>` stripped; `2>&1` isn't in the operator list
        // (it's a shell-quote literal), so it survives.
        assert_eq!(out, vec!["ls", "file.txt", "log", "2>&1"]);
    }

    #[test]
    fn filter_preserves_non_operator_tokens() {
        let input = vec!["git", "status", "--short"];
        let out = filter_control_operators(input);
        assert_eq!(out, vec!["git", "status", "--short"]);
    }

    #[test]
    fn filter_accepts_string_and_str_refs() {
        // Contract: caller can pass Vec<String>, &[&str], or
        // IntoIterator<&String> — all work.
        let owned: Vec<String> = vec!["a".into(), "&&".into(), "b".into()];
        assert_eq!(filter_control_operators(&owned), vec!["a", "b"]);
        let borrowed: &[&str] = &["a", "||", "b"];
        assert_eq!(filter_control_operators(borrowed), vec!["a", "b"]);
    }

    #[test]
    fn separator_sets_cover_expected_operators() {
        // Pin the set contents so accidental edits to the
        // operator lists are caught.
        for op in ["&&", "||", ";", ";;", "|"] {
            assert!(
                COMMAND_LIST_SEPARATORS.contains(&op),
                "{} should be in COMMAND_LIST_SEPARATORS",
                op
            );
        }
        for op in ["&&", "||", ";", ";;", "|", ">&", ">", ">>"] {
            assert!(
                ALL_SUPPORTED_CONTROL_OPERATORS.contains(&op),
                "{} should be in ALL_SUPPORTED_CONTROL_OPERATORS",
                op
            );
        }
    }
}
