//! Shell command tokeniser + quoter + security sanity checks.
//!
//! Port of `src/utils/bash/shellQuote.ts`. The TS layer wraps the
//! npm `shell-quote` library; on the Rust side we hand-roll the
//! tokeniser to avoid a new dependency, matching bash quote/escape
//! semantics and producing a `ParseEntry` stream compatible with
//! the call sites in `utils/bash/commands.ts`, `bashSecurity.ts`,
//! `readOnlyValidation.ts`, and friends.
//!
//! Tokeniser scope:
//! - Word splitting on unquoted whitespace.
//! - Single quotes: literal through to the next `'`.
//! - Double quotes: literal except `\` (before `"`, `\`, `$`, `` ` ``),
//!   `$`, and `` ` ``. `$` and `` ` `` are emitted as part of the token;
//!   we do not expand them.
//! - Backslash outside any quote: literal next char (newline joins
//!   lines — emitted as empty).
//! - Operators: `|`, `||`, `|&`, `&`, `&&`, `&>`, `;`, `;;`, `<`, `<<`,
//!   `<<-`, `<<<`, `>`, `>>`, `>&`, `(`, `)`.
//! - Comments: `#` → entry is `Comment(rest-of-line)`.
//!
//! Does NOT expand: environment variables, command substitution,
//! parameter expansion, arithmetic expansion, brace expansion,
//! globs, heredocs. Call sites that need expansion must layer on
//! top.

use std::fmt;

/// One token produced by the shell tokeniser. `Literal` is a quoted
/// or bare word; `Op` is a control-flow/redirection operator;
/// `Comment` is the trailing `# ...` text (without the `#`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseEntry {
    Literal(String),
    Op(String),
    Comment(String),
}

impl ParseEntry {
    pub fn as_literal(&self) -> Option<&str> {
        if let ParseEntry::Literal(s) = self {
            Some(s.as_str())
        } else {
            None
        }
    }

    pub fn is_op(&self) -> bool {
        matches!(self, ParseEntry::Op(_))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellParseError {
    UnterminatedSingleQuote,
    UnterminatedDoubleQuote,
    TrailingBackslash,
}

impl fmt::Display for ShellParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShellParseError::UnterminatedSingleQuote => {
                f.write_str("unterminated single quote")
            }
            ShellParseError::UnterminatedDoubleQuote => {
                f.write_str("unterminated double quote")
            }
            ShellParseError::TrailingBackslash => {
                f.write_str("trailing backslash without next character")
            }
        }
    }
}

impl std::error::Error for ShellParseError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellParseResult {
    Ok(Vec<ParseEntry>),
    Err(String),
}

impl ShellParseResult {
    pub fn is_ok(&self) -> bool {
        matches!(self, ShellParseResult::Ok(_))
    }
    pub fn tokens(&self) -> Option<&[ParseEntry]> {
        if let ShellParseResult::Ok(v) = self {
            Some(v)
        } else {
            None
        }
    }
    pub fn error(&self) -> Option<&str> {
        if let ShellParseResult::Err(e) = self {
            Some(e.as_str())
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellQuoteResult {
    Ok(String),
    Err(String),
}

impl ShellQuoteResult {
    pub fn is_ok(&self) -> bool {
        matches!(self, ShellQuoteResult::Ok(_))
    }
    pub fn quoted(&self) -> Option<&str> {
        if let ShellQuoteResult::Ok(s) = self {
            Some(s.as_str())
        } else {
            None
        }
    }
    pub fn error(&self) -> Option<&str> {
        if let ShellQuoteResult::Err(e) = self {
            Some(e.as_str())
        } else {
            None
        }
    }
}

/// Tokenise a shell command into `ParseEntry` values. Returns
/// `ShellParseResult::Err` for unterminated quotes / trailing `\`.
/// Equivalent to the TS `tryParseShellCommand` wrapper — logs are
/// caller's responsibility on Rust side.
pub fn try_parse_shell_command(cmd: &str) -> ShellParseResult {
    match tokenise(cmd) {
        Ok(v) => ShellParseResult::Ok(v),
        Err(e) => ShellParseResult::Err(e.to_string()),
    }
}

fn tokenise(cmd: &str) -> Result<Vec<ParseEntry>, ShellParseError> {
    let bytes = cmd.as_bytes();
    let mut i = 0;
    let mut out: Vec<ParseEntry> = Vec::new();
    let n = bytes.len();

    while i < n {
        let c = bytes[i];

        // Skip unquoted whitespace.
        if c == b' ' || c == b'\t' || c == b'\n' {
            i += 1;
            continue;
        }

        // Comment: # to end of line (and end-of-string).
        if c == b'#' {
            let start = i + 1;
            let mut j = start;
            while j < n && bytes[j] != b'\n' {
                j += 1;
            }
            let text = std::str::from_utf8(&bytes[start..j])
                .unwrap_or("")
                .to_string();
            out.push(ParseEntry::Comment(text));
            i = j;
            continue;
        }

        // Operators.
        if let Some((op, len)) = read_op(bytes, i) {
            out.push(ParseEntry::Op(op));
            i += len;
            continue;
        }

        // Word.
        let (word, consumed) = read_word(bytes, i)?;
        out.push(ParseEntry::Literal(word));
        i += consumed;
    }

    Ok(out)
}

fn read_op(bytes: &[u8], i: usize) -> Option<(String, usize)> {
    let n = bytes.len();
    let c = bytes[i];
    let c2 = if i + 1 < n { Some(bytes[i + 1]) } else { None };
    let c3 = if i + 2 < n { Some(bytes[i + 2]) } else { None };

    match (c, c2, c3) {
        (b'<', Some(b'<'), Some(b'-')) => Some(("<<-".into(), 3)),
        (b'<', Some(b'<'), Some(b'<')) => Some(("<<<".into(), 3)),
        (b'<', Some(b'<'), _) => Some(("<<".into(), 2)),
        (b'>', Some(b'>'), _) => Some((">>".into(), 2)),
        (b'>', Some(b'&'), _) => Some((">&".into(), 2)),
        (b'&', Some(b'&'), _) => Some(("&&".into(), 2)),
        (b'&', Some(b'>'), _) => Some(("&>".into(), 2)),
        (b'|', Some(b'|'), _) => Some(("||".into(), 2)),
        (b'|', Some(b'&'), _) => Some(("|&".into(), 2)),
        (b';', Some(b';'), _) => Some((";;".into(), 2)),
        (b'|', _, _) => Some(("|".into(), 1)),
        (b'&', _, _) => Some(("&".into(), 1)),
        (b';', _, _) => Some((";".into(), 1)),
        (b'<', _, _) => Some(("<".into(), 1)),
        (b'>', _, _) => Some((">".into(), 1)),
        (b'(', _, _) => Some(("(".into(), 1)),
        (b')', _, _) => Some((")".into(), 1)),
        _ => None,
    }
}

fn read_word(bytes: &[u8], start: usize) -> Result<(String, usize), ShellParseError> {
    let n = bytes.len();
    let mut i = start;
    let mut buf: Vec<u8> = Vec::new();

    while i < n {
        let c = bytes[i];

        // Whitespace / operator / comment terminates.
        if c == b' ' || c == b'\t' || c == b'\n' {
            break;
        }
        if c == b'#' {
            // `#` mid-word is literal (bash only treats `#` at start
            // of a word as a comment). We already handled word-start
            // above; keep consuming.
            buf.push(c);
            i += 1;
            continue;
        }
        if matches!(c, b'|' | b'&' | b';' | b'<' | b'>' | b'(' | b')') {
            break;
        }

        if c == b'\\' {
            if i + 1 >= n {
                return Err(ShellParseError::TrailingBackslash);
            }
            let next = bytes[i + 1];
            if next == b'\n' {
                // Line continuation: drop both.
                i += 2;
                continue;
            }
            buf.push(next);
            i += 2;
            continue;
        }

        if c == b'\'' {
            i += 1;
            let sq_start = i;
            while i < n && bytes[i] != b'\'' {
                i += 1;
            }
            if i >= n {
                return Err(ShellParseError::UnterminatedSingleQuote);
            }
            buf.extend_from_slice(&bytes[sq_start..i]);
            i += 1;
            continue;
        }

        if c == b'"' {
            i += 1;
            while i < n && bytes[i] != b'"' {
                if bytes[i] == b'\\' && i + 1 < n {
                    let next = bytes[i + 1];
                    // In double quotes, `\` only escapes `"`, `\`,
                    // `$`, `` ` ``, and newline. Otherwise both chars
                    // are literal.
                    if matches!(next, b'"' | b'\\' | b'$' | b'`') {
                        buf.push(next);
                        i += 2;
                        continue;
                    }
                    if next == b'\n' {
                        i += 2;
                        continue;
                    }
                    buf.push(b'\\');
                    buf.push(next);
                    i += 2;
                    continue;
                }
                buf.push(bytes[i]);
                i += 1;
            }
            if i >= n {
                return Err(ShellParseError::UnterminatedDoubleQuote);
            }
            i += 1;
            continue;
        }

        buf.push(c);
        i += 1;
    }

    let word = String::from_utf8(buf).unwrap_or_default();
    Ok((word, i - start))
}

/// Single-pass check that the raw command string has no unterminated
/// single or double quote. Used by `has_malformed_tokens`.
fn quotes_balanced(command: &str) -> bool {
    let bytes = command.as_bytes();
    let n = bytes.len();
    let mut i = 0;
    let mut in_single = false;
    let mut in_double = false;
    let mut single_count = 0usize;
    let mut double_count = 0usize;

    while i < n {
        let c = bytes[i];
        if c == b'\\' && !in_single {
            i += 2;
            continue;
        }
        if c == b'"' && !in_single {
            double_count += 1;
            in_double = !in_double;
        } else if c == b'\'' && !in_double {
            single_count += 1;
            in_single = !in_single;
        }
        i += 1;
    }

    double_count % 2 == 0 && single_count % 2 == 0
}

/// Mirror of `hasMalformedTokens(command, parsed)`. Flags commands
/// where the tokeniser produced entries with unbalanced braces /
/// parens / brackets / quotes, or where the raw string has
/// unterminated quotes.
pub fn has_malformed_tokens(command: &str, parsed: &[ParseEntry]) -> bool {
    if !quotes_balanced(command) {
        return true;
    }
    for entry in parsed {
        let s = match entry {
            ParseEntry::Literal(s) => s,
            _ => continue,
        };

        if count_balanced(s, '{', '}') != 0 {
            return true;
        }
        if count_balanced(s, '(', ')') != 0 {
            return true;
        }
        if count_balanced(s, '[', ']') != 0 {
            return true;
        }
        if count_unescaped(s, '"') % 2 != 0 {
            return true;
        }
        if count_unescaped(s, '\'') % 2 != 0 {
            return true;
        }
    }
    false
}

fn count_balanced(s: &str, open: char, close: char) -> i32 {
    let mut n = 0i32;
    for c in s.chars() {
        if c == open {
            n += 1;
        } else if c == close {
            n -= 1;
        }
    }
    n
}

fn count_unescaped(s: &str, target: char) -> usize {
    let chars: Vec<char> = s.chars().collect();
    let mut count = 0usize;
    for i in 0..chars.len() {
        if chars[i] != target {
            continue;
        }
        let prev_is_backslash = i > 0 && chars[i - 1] == '\\';
        if !prev_is_backslash {
            count += 1;
        }
    }
    count
}

/// Detects commands exploiting the TS `shell-quote` library's
/// backslash-inside-single-quote bug. The Rust tokeniser above is
/// correct, but the check is still load-bearing: callers may pipe
/// the command to tooling that uses the JS parser, or want to
/// refuse adversarial patterns outright.
///
/// Walks the command with correct bash single-quote semantics.
/// When a single quote closes, inspects the trailing run of
/// backslashes before it:
/// - Odd count → always a bug.
/// - Even count → a bug only when a later `'` exists in the string
///   (the JS chunker's regex can consume it as a false close).
pub fn has_shell_quote_single_quote_bug(command: &str) -> bool {
    let chars: Vec<char> = command.chars().collect();
    let n = chars.len();
    let mut i = 0;
    let mut in_single = false;
    let mut in_double = false;

    while i < n {
        let c = chars[i];

        if c == '\\' && !in_single {
            i += 2;
            continue;
        }

        if c == '"' && !in_single {
            in_double = !in_double;
            i += 1;
            continue;
        }

        if c == '\'' && !in_double {
            in_single = !in_single;
            if !in_single {
                // Just closed a single-quoted string at index `i`.
                let mut backslash_count = 0usize;
                let mut j = i as isize - 1;
                while j >= 0 && chars[j as usize] == '\\' {
                    backslash_count += 1;
                    j -= 1;
                }
                if backslash_count > 0 && backslash_count % 2 == 1 {
                    return true;
                }
                if backslash_count > 0
                    && backslash_count % 2 == 0
                    && chars[i + 1..].iter().any(|&ch| ch == '\'')
                {
                    return true;
                }
            }
            i += 1;
            continue;
        }

        i += 1;
    }

    false
}

/// Rust equivalent of `tryQuoteShellArgs`. Quotes each arg for safe
/// shell reuse. Follows shell-quote's rules:
/// - Empty → `''`.
/// - Matches `^[A-Za-z0-9_./:=@%+,-]+$` → bare.
/// - Otherwise → single-quote and replace embedded `'` with `'\''`.
pub fn try_quote_shell_args<I, S>(args: I) -> ShellQuoteResult
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let parts: Vec<String> = args
        .into_iter()
        .map(|a| quote_one(a.as_ref()))
        .collect();
    ShellQuoteResult::Ok(parts.join(" "))
}

fn quote_one(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || "_./:=@%+,-".contains(c))
    {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

/// Convenience wrapper matching TS `quote(args)`. On the Rust side
/// inputs are already `&str`, so there is no lenient JSON fallback
/// path — we simply delegate to `try_quote_shell_args`.
pub fn quote<I, S>(args: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    match try_quote_shell_args(args) {
        ShellQuoteResult::Ok(s) => s,
        ShellQuoteResult::Err(_) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(cmd: &str) -> Vec<ParseEntry> {
        match try_parse_shell_command(cmd) {
            ShellParseResult::Ok(v) => v,
            ShellParseResult::Err(e) => panic!("parse failed: {e}"),
        }
    }

    #[test]
    fn splits_simple_words() {
        let out = parse("echo hello world");
        assert_eq!(
            out,
            vec![
                ParseEntry::Literal("echo".into()),
                ParseEntry::Literal("hello".into()),
                ParseEntry::Literal("world".into()),
            ]
        );
    }

    #[test]
    fn single_quotes_are_literal() {
        let out = parse("echo 'a $b c'");
        assert_eq!(
            out,
            vec![
                ParseEntry::Literal("echo".into()),
                ParseEntry::Literal("a $b c".into()),
            ]
        );
    }

    #[test]
    fn double_quotes_keep_dollars_literal() {
        let out = parse(r#"echo "a $b c""#);
        assert_eq!(
            out,
            vec![
                ParseEntry::Literal("echo".into()),
                ParseEntry::Literal("a $b c".into()),
            ]
        );
    }

    #[test]
    fn backslash_escapes_in_double_quotes() {
        let out = parse(r#"echo "a\"b""#);
        assert_eq!(
            out,
            vec![
                ParseEntry::Literal("echo".into()),
                ParseEntry::Literal("a\"b".into()),
            ]
        );
    }

    #[test]
    fn unterminated_single_is_error() {
        match try_parse_shell_command("echo 'hi") {
            ShellParseResult::Err(_) => {}
            _ => panic!("expected error"),
        }
    }

    #[test]
    fn unterminated_double_is_error() {
        match try_parse_shell_command(r#"echo "hi"#) {
            ShellParseResult::Err(_) => {}
            _ => panic!("expected error"),
        }
    }

    #[test]
    fn operators_emitted_as_op_entries() {
        let out = parse("a; b && c || d | e");
        assert_eq!(
            out,
            vec![
                ParseEntry::Literal("a".into()),
                ParseEntry::Op(";".into()),
                ParseEntry::Literal("b".into()),
                ParseEntry::Op("&&".into()),
                ParseEntry::Literal("c".into()),
                ParseEntry::Op("||".into()),
                ParseEntry::Literal("d".into()),
                ParseEntry::Op("|".into()),
                ParseEntry::Literal("e".into()),
            ]
        );
    }

    #[test]
    fn redirects_recognised() {
        let out = parse("cmd > out 2>> log <<< text");
        assert_eq!(
            out,
            vec![
                ParseEntry::Literal("cmd".into()),
                ParseEntry::Op(">".into()),
                ParseEntry::Literal("out".into()),
                ParseEntry::Literal("2".into()),
                ParseEntry::Op(">>".into()),
                ParseEntry::Literal("log".into()),
                ParseEntry::Op("<<<".into()),
                ParseEntry::Literal("text".into()),
            ]
        );
    }

    #[test]
    fn comment_captured_as_comment_entry() {
        let out = parse("echo hi # trailing note");
        assert_eq!(
            out,
            vec![
                ParseEntry::Literal("echo".into()),
                ParseEntry::Literal("hi".into()),
                ParseEntry::Comment(" trailing note".into()),
            ]
        );
    }

    #[test]
    fn hash_midword_is_literal() {
        let out = parse("abc#def");
        assert_eq!(out, vec![ParseEntry::Literal("abc#def".into())]);
    }

    #[test]
    fn line_continuation_consumed() {
        let out = parse("echo a\\\nb");
        assert_eq!(
            out,
            vec![
                ParseEntry::Literal("echo".into()),
                ParseEntry::Literal("ab".into()),
            ]
        );
    }

    #[test]
    fn has_malformed_detects_unterminated_raw_quote() {
        let parsed = parse("echo hi");
        assert!(has_malformed_tokens("echo 'hi", &parsed) == true);
    }

    #[test]
    fn has_malformed_flags_unbalanced_brace() {
        let parsed = vec![ParseEntry::Literal("{hi:\"hi".into())];
        assert!(has_malformed_tokens("echo {\"hi\":\"hi;evil\"}", &parsed));
    }

    #[test]
    fn has_malformed_clears_balanced() {
        let parsed = parse("echo hi");
        assert!(!has_malformed_tokens("echo hi", &parsed));
    }

    #[test]
    fn single_quote_bug_odd_backslash() {
        assert!(has_shell_quote_single_quote_bug("'\\'"));
        assert!(has_shell_quote_single_quote_bug("'abc\\'"));
    }

    #[test]
    fn single_quote_bug_even_backslash_with_later_quote() {
        assert!(has_shell_quote_single_quote_bug(
            "git ls-remote 'safe\\\\' '--upload-pack=evil' 'repo'"
        ));
    }

    #[test]
    fn single_quote_bug_even_backslash_no_later_quote() {
        // '\\' alone: shell-quote backtracks — not flagged.
        assert!(!has_shell_quote_single_quote_bug("'\\\\'"));
    }

    #[test]
    fn single_quote_bug_normal_commands_clear() {
        assert!(!has_shell_quote_single_quote_bug("echo hi"));
        assert!(!has_shell_quote_single_quote_bug("echo 'a b c'"));
        assert!(!has_shell_quote_single_quote_bug("echo \"a b c\""));
    }

    #[test]
    fn quote_bare_word_unquoted() {
        assert_eq!(quote(["echo", "hello"]), "echo hello");
    }

    #[test]
    fn quote_empty_arg() {
        assert_eq!(quote([""]), "''");
    }

    #[test]
    fn quote_space_gets_single_quoted() {
        assert_eq!(quote(["a b"]), "'a b'");
    }

    #[test]
    fn quote_embedded_single_quote_escaped() {
        assert_eq!(quote(["it's"]), "'it'\\''s'");
    }

    #[test]
    fn quote_dollar_expansion_suppressed() {
        // Critical: $(whoami) MUST end up single-quoted, not
        // double-quoted. Double quotes allow shell expansion.
        let q = quote(["echo", "$(whoami)"]);
        assert_eq!(q, "echo '$(whoami)'");
        assert!(!q.contains('"'));
    }

    #[test]
    fn round_trip_parse_after_quote() {
        let args = ["echo", "hi there", "$x"];
        let quoted = quote(args);
        let parsed = parse(&quoted);
        let literals: Vec<&str> = parsed
            .iter()
            .filter_map(|e| e.as_literal())
            .collect();
        assert_eq!(literals, vec!["echo", "hi there", "$x"]);
    }
}
