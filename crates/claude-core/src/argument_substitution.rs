//! `$ARGUMENTS` placeholder expansion for slash-commands + skills.
//!
//! Port of TS `src/utils/argumentSubstitution.ts`. The slash-command
//! and skill loaders call this when the user invokes `/foo arg1 arg2`
//! — the prompt template embeds `$ARGUMENTS`, `$ARGUMENTS[0]`, `$0`,
//! or named placeholders from frontmatter, and this module expands
//! them against the user's input.
//!
//! Behaviour (matches TS):
//! - `None` args → return content unchanged.
//! - `""` args → still run the replacement pass (empty string is a
//!   valid invocation with no args).
//! - Unfilled indexed positions expand to `""`.
//! - Named args from frontmatter map to the same positions as the
//!   parsed input tokens (name[i] ↔ parsed[i]).
//! - When NO placeholder matched and `append_if_no_placeholder` is
//!   true, append `\n\nARGUMENTS: {args}` — but only if `args` is
//!   non-empty. A no-arg invocation leaves content untouched.

use crate::shell_quote::{try_parse_shell_command, ParseEntry, ShellParseResult};
use regex::Regex;

/// Tokenise `args` using shell-quote semantics. Falls back to a
/// simple whitespace split on tokeniser error. Variable-looking
/// tokens (`$KEY`) are preserved literally — we don't expand env
/// vars when splitting user-supplied arguments.
pub fn parse_arguments(args: &str) -> Vec<String> {
    if args.trim().is_empty() {
        return Vec::new();
    }
    match try_parse_shell_command(args) {
        ShellParseResult::Ok(tokens) => tokens
            .into_iter()
            .filter_map(|entry| match entry {
                ParseEntry::Literal(s) => Some(s),
                _ => None,
            })
            .collect(),
        ShellParseResult::Err(_) => args
            .split_whitespace()
            .map(|s| s.to_string())
            .collect(),
    }
}

/// Normalise argument names from frontmatter. Accepts a
/// whitespace-separated string; callers that store a YAML list
/// should pass it pre-joined. Filters empty entries and purely
/// numeric names (those collide with the `$0`, `$1` shorthand).
pub fn parse_argument_names_from_str(argument_names: &str) -> Vec<String> {
    argument_names
        .split_whitespace()
        .filter(|s| is_valid_arg_name(s))
        .map(|s| s.to_string())
        .collect()
}

/// Normalise argument names already split into a list. Same
/// filtering as `parse_argument_names_from_str`.
pub fn parse_argument_names_from_list<I, S>(argument_names: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    argument_names
        .into_iter()
        .filter(|name| is_valid_arg_name(name.as_ref()))
        .map(|name| name.as_ref().to_string())
        .collect()
}

fn is_valid_arg_name(name: &str) -> bool {
    let trimmed = name.trim();
    !trimmed.is_empty() && !trimmed.bytes().all(|b| b.is_ascii_digit())
}

/// Return true if the char at `end` would extend a `$name` or `$n`
/// placeholder — i.e. a word char (plus `_`) or `[` when
/// `bracket_breaks` is true (named args are also broken by `[` so
/// `$name[0]` isn't mistaken for bare `$name`; bare `$n` shorthand
/// is only broken by word chars — `$0[0]` is invalid syntax anyway).
fn is_boundary_breaker_at(s: &str, end: usize, bracket_breaks: bool) -> bool {
    let tail = &s[end..];
    let Some(c) = tail.chars().next() else {
        return false;
    };
    if bracket_breaks && c == '[' {
        return true;
    }
    c.is_alphanumeric() || c == '_'
}

/// Build a hint string showing the remaining unfilled argument
/// placeholders, e.g. `[arg2] [arg3]`. Returns `None` if every name
/// has already been filled by the typed args.
pub fn generate_progressive_argument_hint(
    arg_names: &[String],
    typed_args: &[String],
) -> Option<String> {
    let remaining = arg_names.get(typed_args.len()..).unwrap_or(&[]);
    if remaining.is_empty() {
        return None;
    }
    Some(
        remaining
            .iter()
            .map(|n| format!("[{n}]"))
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// Expand `$ARGUMENTS`, `$ARGUMENTS[n]`, `$n`, and named placeholders
/// against `args`. `argument_names` maps names to positional slots.
///
/// `append_if_no_placeholder`: when true and no substitution occurred,
/// append `\n\nARGUMENTS: {args}` — only if `args` is non-empty.
pub fn substitute_arguments(
    content: &str,
    args: Option<&str>,
    append_if_no_placeholder: bool,
    argument_names: &[String],
) -> String {
    let Some(args) = args else {
        return content.to_string();
    };

    let parsed = parse_arguments(args);
    let original = content.to_string();
    let mut content = original.clone();

    for (i, name) in argument_names.iter().enumerate() {
        if name.is_empty() {
            continue;
        }
        // TS lookahead was `(?![\[\w])` — "next char not in [ or
        // word". `regex` has no lookahead, so match `$name` and
        // inspect the following char manually.
        let pattern = format!(r"\${}", regex::escape(name));
        let re = Regex::new(&pattern).expect("valid named-arg regex");
        let replacement_val = parsed.get(i).cloned().unwrap_or_default();
        let mut out = String::with_capacity(content.len());
        let mut last = 0usize;
        for m in re.find_iter(&content) {
            if is_boundary_breaker_at(&content, m.end(), true) {
                out.push_str(&content[last..m.end()]);
            } else {
                out.push_str(&content[last..m.start()]);
                out.push_str(&replacement_val);
            }
            last = m.end();
        }
        out.push_str(&content[last..]);
        content = out;
    }

    // $ARGUMENTS[n]
    let re_indexed = Regex::new(r"\$ARGUMENTS\[(\d+)\]").unwrap();
    content = re_indexed
        .replace_all(&content, |caps: &regex::Captures| {
            let idx: usize = caps[1].parse().unwrap_or(usize::MAX);
            parsed.get(idx).cloned().unwrap_or_default()
        })
        .into_owned();

    // $n shorthand — skip when followed by a word char (so `$0abc`
    // is not treated as placeholder + `abc`).
    let re_short = Regex::new(r"\$(\d+)").unwrap();
    let mut out = String::with_capacity(content.len());
    let mut last = 0usize;
    for caps in re_short.captures_iter(&content) {
        let m = caps.get(0).unwrap();
        let idx: usize = caps[1].parse().unwrap_or(usize::MAX);
        if is_boundary_breaker_at(&content, m.end(), false) {
            out.push_str(&content[last..m.end()]);
        } else {
            let val = parsed.get(idx).cloned().unwrap_or_default();
            out.push_str(&content[last..m.start()]);
            out.push_str(&val);
        }
        last = m.end();
    }
    out.push_str(&content[last..]);
    content = out;

    // $ARGUMENTS with the raw string (last so earlier patterns win).
    content = content.replace("$ARGUMENTS", args);

    if content == original && append_if_no_placeholder && !args.is_empty() {
        content.push_str("\n\nARGUMENTS: ");
        content.push_str(args);
    }

    content
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parse_arguments_whitespace_split() {
        assert_eq!(parse_arguments("foo bar baz"), vec!["foo", "bar", "baz"]);
    }

    #[test]
    fn parse_arguments_handles_quoted_strings() {
        assert_eq!(
            parse_arguments(r#"foo "hello world" baz"#),
            vec!["foo", "hello world", "baz"]
        );
        assert_eq!(
            parse_arguments("foo 'hello world' baz"),
            vec!["foo", "hello world", "baz"]
        );
    }

    #[test]
    fn parse_arguments_empty_returns_empty() {
        assert_eq!(parse_arguments(""), Vec::<String>::new());
        assert_eq!(parse_arguments("   "), Vec::<String>::new());
    }

    #[test]
    fn parse_arguments_falls_back_on_tokenise_error() {
        // Unterminated quote → whitespace split fallback.
        assert_eq!(parse_arguments("foo 'bar"), vec!["foo", "'bar"]);
    }

    #[test]
    fn parse_argument_names_str_filters_numeric() {
        assert_eq!(
            parse_argument_names_from_str("foo 1 bar 42 baz"),
            vec!["foo", "bar", "baz"]
        );
    }

    #[test]
    fn parse_argument_names_list_filters_empty() {
        assert_eq!(
            parse_argument_names_from_list(vec!["foo", "", "bar", "   "]),
            vec!["foo", "bar"]
        );
    }

    #[test]
    fn progressive_hint_returns_remaining() {
        let arg_names = names(&["a", "b", "c"]);
        let typed = names(&["x"]);
        assert_eq!(
            generate_progressive_argument_hint(&arg_names, &typed),
            Some("[b] [c]".into())
        );
    }

    #[test]
    fn progressive_hint_all_filled_is_none() {
        let arg_names = names(&["a", "b"]);
        let typed = names(&["x", "y"]);
        assert_eq!(
            generate_progressive_argument_hint(&arg_names, &typed),
            None
        );
    }

    #[test]
    fn sub_none_is_unchanged() {
        assert_eq!(substitute_arguments("hi", None, true, &[]), "hi");
    }

    #[test]
    fn sub_empty_args_runs_replacement() {
        // Empty string is a valid no-arg invocation — $ARGUMENTS
        // should expand to "" and no ARGUMENTS tail should be
        // appended.
        let out =
            substitute_arguments("before $ARGUMENTS after", Some(""), true, &[]);
        assert_eq!(out, "before  after");
    }

    #[test]
    fn sub_dollar_arguments_full_string() {
        let out = substitute_arguments(
            "cmd: $ARGUMENTS",
            Some("hello world"),
            false,
            &[],
        );
        assert_eq!(out, "cmd: hello world");
    }

    #[test]
    fn sub_dollar_arguments_indexed() {
        let out = substitute_arguments(
            "a=$ARGUMENTS[0], b=$ARGUMENTS[1], c=$ARGUMENTS[2]",
            Some("one two"),
            false,
            &[],
        );
        assert_eq!(out, "a=one, b=two, c=");
    }

    #[test]
    fn sub_shorthand_indexed() {
        let out =
            substitute_arguments("$0 and $1", Some("alpha beta"), false, &[]);
        assert_eq!(out, "alpha and beta");
    }

    #[test]
    fn sub_shorthand_not_followed_by_wordchar() {
        // `$0abc` should NOT be treated as placeholder + "abc".
        let out = substitute_arguments("$0abc", Some("x"), false, &[]);
        assert_eq!(out, "$0abc");
    }

    #[test]
    fn sub_named_arguments() {
        let out = substitute_arguments(
            "target=$target, branch=$branch",
            Some("main feature"),
            false,
            &names(&["target", "branch"]),
        );
        assert_eq!(out, "target=main, branch=feature");
    }

    #[test]
    fn sub_named_arg_not_substring_of_longer_name() {
        // `$foo` shouldn't match inside `$foobar`.
        let out = substitute_arguments(
            "$foobar and $foo",
            Some("A"),
            false,
            &names(&["foo"]),
        );
        assert_eq!(out, "$foobar and A");
    }

    #[test]
    fn sub_appends_when_no_placeholder_and_args_nonempty() {
        let out =
            substitute_arguments("do a thing", Some("alpha beta"), true, &[]);
        assert!(out.ends_with("\n\nARGUMENTS: alpha beta"));
    }

    #[test]
    fn sub_does_not_append_when_args_empty() {
        let out = substitute_arguments("no placeholders", Some(""), true, &[]);
        assert_eq!(out, "no placeholders");
    }

    #[test]
    fn sub_does_not_append_when_placeholder_matched() {
        let out = substitute_arguments("got $ARGUMENTS", Some("x"), true, &[]);
        assert_eq!(out, "got x");
    }

    #[test]
    fn sub_missing_indexed_yields_empty() {
        let out = substitute_arguments("$0 $1 $2", Some("only"), false, &[]);
        assert_eq!(out, "only  ");
    }
}
