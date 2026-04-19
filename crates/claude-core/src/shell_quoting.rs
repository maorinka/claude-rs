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
static HEREDOC_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"<<-?\s*(?:'(?:\w+)'|"(?:\w+)"|\\(?:\w+)|(?:\w+))"#).unwrap()
});

/// Bit-shift operator patterns that must NOT count as heredocs.
/// TS at `shellQuoting.ts:12-16`. Three distinct forms — each
/// arithmetic context where `<<` means left-shift.
static BIT_SHIFT_INT: Lazy<Regex> = Lazy::new(|| Regex::new(r#"\d\s*<<\s*\d"#).unwrap());
static BIT_SHIFT_BRACKETS: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"\[\[\s*\d+\s*<<\s*\d+\s*\]\]"#).unwrap());
static BIT_SHIFT_ARITH: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"\$\(\(.*<<.*\)\)"#).unwrap());

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
static NUL_REDIRECT_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?i)(\d?&?>+\s*)nul(\s|$|[|&;)\n])"#).unwrap()
});

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
    SINGLE_QUOTE_MULTILINE.is_match(command)
        || DOUBLE_QUOTE_MULTILINE.is_match(command)
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
        assert_eq!(
            rewrite_windows_null_redirect("ls >nul"),
            "ls >/dev/null"
        );
        assert_eq!(
            rewrite_windows_null_redirect("ls 2>nul"),
            "ls 2>/dev/null"
        );
        assert_eq!(
            rewrite_windows_null_redirect("ls &>nul"),
            "ls &>/dev/null"
        );
        assert_eq!(
            rewrite_windows_null_redirect("ls >>nul"),
            "ls >>/dev/null"
        );
    }

    #[test]
    fn rewrite_nul_case_insensitive() {
        assert_eq!(
            rewrite_windows_null_redirect("ls > NUL"),
            "ls > /dev/null"
        );
        assert_eq!(
            rewrite_windows_null_redirect("ls >Nul"),
            "ls >/dev/null"
        );
    }

    #[test]
    fn rewrite_nul_guards_against_false_positives() {
        // `>null`, `>nullable`, `>nul.txt` must stay untouched.
        assert_eq!(
            rewrite_windows_null_redirect("echo >null"),
            "echo >null"
        );
        assert_eq!(
            rewrite_windows_null_redirect("ls >nul.txt"),
            "ls >nul.txt"
        );
        assert_eq!(
            rewrite_windows_null_redirect("cat nul.txt"),
            "cat nul.txt"
        );
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
        assert_eq!(
            rewrite_windows_null_redirect("ls >nul"),
            "ls >/dev/null"
        );
    }
}
