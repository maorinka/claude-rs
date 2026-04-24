//! Dangerous-sed-command detector.
//!
//! The TS `sedEditParser.ts` + `sedValidation.ts` together total ~1,000
//! LOC of allowlist + pattern detection. Porting the full positive
//! allowlist (the "this sed is definitely read-only" path) is its own
//! effort; this module ports the security-critical denylist: the set of
//! sed constructs that ALWAYS require explicit user approval because they
//! write to the filesystem or execute subprocesses.
//!
//! Use `has_dangerous_sed_construct(command)` from the BashTool permission
//! layer after the broader read-only classifier returns Unknown — it
//! flips the decision to "ask" whenever the command contains:
//!   - `-i` (in-place edit)
//!   - `w`/`W` commands (write file)
//!   - `r` (read file)
//!   - `e` (execute)
//!   - `s/.../.../w`, `s/.../.../e` flag suffixes
//!   - `y` (transliterate) paired with any of the above letters

use regex_lite::Regex;
use std::sync::OnceLock;

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("sed pattern compiles")
}

/// Does the full invocation include `sed -i …` somewhere?
/// Covers `-i`, `-i ''`, `-i.bak`, `-i''` and the long form `--in-place`.
fn has_in_place_flag(command: &str) -> bool {
    static CELL: OnceLock<Regex> = OnceLock::new();
    let r = CELL.get_or_init(|| re(r"\bsed\b[^;&|\n]*(\s|^)(--in-place\b|-[a-zA-Z]*i[a-zA-Z]*)"));
    r.is_match(command)
}

/// Check an individual sed expression (the body of a quoted block after
/// `sed …`) for the commands that write to disk or execute subprocesses.
/// Mirrors the core of TS `containsDangerousOperations`.
fn expression_has_dangerous_op(expr: &str) -> bool {
    static WRITE_W: OnceLock<Regex> = OnceLock::new();
    static WRITE_W_ADDR: OnceLock<Regex> = OnceLock::new();
    static WRITE_W_DOLLAR: OnceLock<Regex> = OnceLock::new();
    static WRITE_W_PATTERN: OnceLock<Regex> = OnceLock::new();
    static WRITE_W_RANGE: OnceLock<Regex> = OnceLock::new();
    static EXEC_E: OnceLock<Regex> = OnceLock::new();
    static EXEC_E_ADDR: OnceLock<Regex> = OnceLock::new();
    static EXEC_E_PATTERN: OnceLock<Regex> = OnceLock::new();
    static SUBST_FLAGS: OnceLock<Regex> = OnceLock::new();
    static Y_CMD: OnceLock<Regex> = OnceLock::new();
    static READ_R: OnceLock<Regex> = OnceLock::new();

    let cmd = expr.trim();

    // `w file` / `W file` at start or after an address.
    let write_w = WRITE_W.get_or_init(|| re(r"^[wW]\s*\S+"));
    let write_w_addr = WRITE_W_ADDR.get_or_init(|| re(r"^\d+\s*[wW]\s*\S+"));
    let write_w_dollar = WRITE_W_DOLLAR.get_or_init(|| re(r"^\$\s*[wW]\s*\S+"));
    let write_w_pattern = WRITE_W_PATTERN.get_or_init(|| re(r"^/[^/]*/[IMim]*\s*[wW]\s*\S+"));
    let write_w_range = WRITE_W_RANGE.get_or_init(|| re(r"^\d+,(\d+|\$)\s*[wW]\s*\S+"));
    if write_w.is_match(cmd)
        || write_w_addr.is_match(cmd)
        || write_w_dollar.is_match(cmd)
        || write_w_pattern.is_match(cmd)
        || write_w_range.is_match(cmd)
    {
        return true;
    }

    // `e` (execute) at start or after address.
    let exec_e = EXEC_E.get_or_init(|| re(r"^e(\s|$)"));
    let exec_e_addr = EXEC_E_ADDR.get_or_init(|| re(r"^\d+\s*e"));
    let exec_e_pattern = EXEC_E_PATTERN.get_or_init(|| re(r"^/[^/]*/[IMim]*\s*e"));
    if exec_e.is_match(cmd) || exec_e_addr.is_match(cmd) || exec_e_pattern.is_match(cmd) {
        return true;
    }

    // `r file` — reads external file into the stream.
    let read_r = READ_R.get_or_init(|| re(r"^r\s+\S+"));
    if read_r.is_match(cmd) {
        return true;
    }

    // Substitution flags: `s/a/b/ge` or `s/a/b/gw out`.
    // regex-lite does not support back-references, so we scan for the
    // common delimiters (/, |, #) and inspect the flag group after the
    // third delimiter. Exotic delimiters fall through — the pattern
    // detectors above already cover the rest.
    let _ = SUBST_FLAGS;
    for delim in ['/', '|', '#'] {
        if let Some(rest) = cmd.strip_prefix(&format!("s{}", delim)) {
            // After first delim, find next unescaped delim for end-of-pattern,
            // then next unescaped delim for end-of-replacement. The remaining
            // chars up to whitespace/;/end are the flags.
            let mut bytes = rest.chars().peekable();
            let mut saw_delims = 1; // already consumed the opening delim
            let mut flags = String::new();
            let mut last_was_backslash = false;
            for ch in bytes.by_ref() {
                if saw_delims >= 3 {
                    if ch.is_whitespace() || ch == ';' || ch == '}' {
                        break;
                    }
                    flags.push(ch);
                    continue;
                }
                if ch == delim && !last_was_backslash {
                    saw_delims += 1;
                }
                last_was_backslash = ch == '\\' && !last_was_backslash;
            }
            if flags.chars().any(|c| matches!(c, 'w' | 'W' | 'e' | 'E')) {
                return true;
            }
        }
    }

    // `y///` transliterate paired with dangerous letters anywhere in the
    // remainder of the command (paranoid but y commands are rare).
    let _ = Y_CMD;
    if cmd.starts_with('y')
        && cmd.len() > 1
        && cmd.chars().nth(1).is_some_and(|c| c != '\\' && c != '\n')
        && cmd[1..].chars().any(|c| matches!(c, 'w' | 'W' | 'e' | 'E'))
    {
        return true;
    }

    false
}

/// Extract the quoted-expression bodies passed to a sed invocation.
/// Cheap parse: look for `-e <expr>` and for the positional expression
/// that follows the last flag before any file arg. Quoted strings may use
/// single or double quotes; unquoted bare expressions are also accepted.
fn extract_expressions(command: &str) -> Vec<String> {
    let mut out = Vec::new();

    // Simplistic tokeniser that respects single/double quotes.
    let mut tokens: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_single = false;
    let mut in_double = false;
    for c in command.chars() {
        match c {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            c if c.is_whitespace() && !in_single && !in_double => {
                if !cur.is_empty() {
                    tokens.push(std::mem::take(&mut cur));
                }
            },
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }

    // Find `sed` then walk tokens: -e <expr> adds expr, first non-flag
    // non-file token is the expression.
    let mut i = 0;
    while i < tokens.len() && tokens[i] != "sed" {
        i += 1;
    }
    if i >= tokens.len() {
        return out;
    }
    i += 1;

    let mut saw_expression = false;
    while i < tokens.len() {
        let tok = &tokens[i];
        if tok == "-e" || tok == "--expression" {
            if let Some(next) = tokens.get(i + 1) {
                out.push(next.clone());
                i += 2;
                continue;
            }
        }
        // Combined flag like -nE / -rn — skip.
        if tok.starts_with('-') && tok.as_str() != "--" {
            i += 1;
            continue;
        }
        // First bare token is the positional expression; the rest are files.
        if !saw_expression {
            out.push(tok.clone());
            saw_expression = true;
        }
        i += 1;
    }

    out
}

/// Does the command contain any sed construct that writes to the
/// filesystem or executes a subprocess? Returns true for commands that
/// the caller must send through the permission-request path.
pub fn has_dangerous_sed_construct(command: &str) -> bool {
    // In-place edit is always dangerous regardless of the script body.
    if has_in_place_flag(command) {
        return true;
    }

    // Otherwise inspect each quoted / bare expression.
    for expr in extract_expressions(command) {
        // Semicolon-split expressions — each subcommand checked independently.
        for part in expr.split(';') {
            if expression_has_dangerous_op(part.trim()) {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_place_flag_caught() {
        assert!(has_dangerous_sed_construct("sed -i 's/a/b/' file.txt"));
        assert!(has_dangerous_sed_construct("sed -i.bak 's/a/b/' file.txt"));
        assert!(has_dangerous_sed_construct("sed --in-place 's/a/b/' f.txt"));
    }

    #[test]
    fn write_command_caught() {
        assert!(has_dangerous_sed_construct("sed -n '1w out.txt' file"));
        assert!(has_dangerous_sed_construct(
            "sed '/pattern/w captured' input"
        ));
    }

    #[test]
    fn execute_command_caught() {
        assert!(has_dangerous_sed_construct("sed 'e rm -rf /' file"));
        assert!(has_dangerous_sed_construct("sed '1e ls' file"));
    }

    #[test]
    fn substitution_write_flag_caught() {
        assert!(has_dangerous_sed_construct("sed 's/a/b/gw outfile' input"));
    }

    #[test]
    fn substitution_execute_flag_caught() {
        assert!(has_dangerous_sed_construct("sed 's/a/b/e' input"));
    }

    #[test]
    fn read_r_caught() {
        assert!(has_dangerous_sed_construct("sed 'r /etc/passwd' file"));
    }

    #[test]
    fn safe_sed_not_flagged() {
        assert!(!has_dangerous_sed_construct("sed -n '1,10p' file.txt"));
        assert!(!has_dangerous_sed_construct("sed 's/foo/bar/g' file.txt"));
        assert!(!has_dangerous_sed_construct("sed '/pattern/p' file.txt"));
        assert!(!has_dangerous_sed_construct("echo hi"));
    }

    #[test]
    fn semicolon_separated_expression_checked() {
        assert!(has_dangerous_sed_construct("sed '1p;2w out' file"));
        assert!(!has_dangerous_sed_construct("sed '1p;2p;3p' file"));
    }
}
