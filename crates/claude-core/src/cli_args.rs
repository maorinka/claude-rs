//! Pre-commander CLI argv helpers.
//!
//! Port of TS `src/utils/cliArgs.ts`. These two helpers run before
//! the main arg parser (clap on Rust, commander on TS) is built:
//! - `eager_parse_cli_flag` lets the config loader read `--settings`
//!   before init() so the chosen settings file affects which clap
//!   definition is installed.
//! - `extract_args_after_double_dash` normalises the Unix `--`
//!   separator when pass-through options leave it sitting in the
//!   positional array rather than consuming it.
//!
//! Both are intentionally tiny and dependency-free so they can be
//! called from `main()` before any global state exists.

/// Find `--flag` (or `--flag=value`) in `argv` and return its value.
///
/// - `--flag=value` → returns `value`.
/// - `--flag value` → returns the next argv entry.
/// - Flag missing or has no value → returns `None`.
///
/// `flag_name` MUST include leading dashes (e.g. `--settings`). The
/// matcher is an exact-string compare; partial/prefix forms are not
/// supported on purpose — this runs before clap's grammar exists.
pub fn eager_parse_cli_flag<'a>(flag_name: &str, argv: &'a [impl AsRef<str>]) -> Option<&'a str> {
    let eq_prefix = format!("{flag_name}=");
    for (i, arg) in argv.iter().enumerate() {
        let arg = arg.as_ref();
        if let Some(rest) = arg.strip_prefix(&eq_prefix) {
            return Some(unsafe_slice_after_prefix(argv, i, rest));
        }
        if arg == flag_name && i + 1 < argv.len() {
            return Some(argv[i + 1].as_ref());
        }
    }
    None
}

// Return `rest` as a slice of the original argv entry at index `i`.
// The `rest` we have is the owned-by-argv string — we just need to
// reborrow it with the lifetime of the slice.
fn unsafe_slice_after_prefix<'a>(argv: &'a [impl AsRef<str>], i: usize, rest: &str) -> &'a str {
    let full = argv[i].as_ref();
    let start = full.len() - rest.len();
    &full[start..]
}

/// Normalise a pass-through `--` separator.
///
/// When clap / commander's pass-through mode leaves `--` as the
/// command positional, the real command is in the first element of
/// the remaining args. This function collapses that case, returning
/// `(command, args_without_leading_command)`.
///
/// When `command_or_value` is anything other than `"--"`, the input
/// is passed through unchanged.
pub fn extract_args_after_double_dash<'a>(
    command_or_value: &'a str,
    args: &'a [String],
) -> (&'a str, Vec<String>) {
    if command_or_value == "--" {
        if let Some((first, rest)) = args.split_first() {
            return (first.as_str(), rest.to_vec());
        }
    }
    (command_or_value, args.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn eager_parses_equals_form() {
        let a = argv(&["bin", "--settings=./foo.json", "--other"]);
        assert_eq!(eager_parse_cli_flag("--settings", &a), Some("./foo.json"));
    }

    #[test]
    fn eager_parses_space_form() {
        let a = argv(&["bin", "--settings", "./foo.json", "--other"]);
        assert_eq!(eager_parse_cli_flag("--settings", &a), Some("./foo.json"));
    }

    #[test]
    fn eager_flag_missing_returns_none() {
        let a = argv(&["bin", "--other", "x"]);
        assert_eq!(eager_parse_cli_flag("--settings", &a), None);
    }

    #[test]
    fn eager_flag_without_value_returns_none() {
        // `--settings` at the tail with no next argv entry.
        let a = argv(&["bin", "--settings"]);
        assert_eq!(eager_parse_cli_flag("--settings", &a), None);
    }

    #[test]
    fn eager_equals_value_can_be_empty() {
        let a = argv(&["bin", "--settings="]);
        assert_eq!(eager_parse_cli_flag("--settings", &a), Some(""));
    }

    #[test]
    fn eager_picks_first_occurrence() {
        let a = argv(&["bin", "--settings=first", "--settings=second"]);
        assert_eq!(eager_parse_cli_flag("--settings", &a), Some("first"));
    }

    #[test]
    fn double_dash_extracts_first_remaining() {
        let args = argv(&["subcmd", "--flag", "arg"]);
        let (cmd, rest) = extract_args_after_double_dash("--", &args);
        assert_eq!(cmd, "subcmd");
        assert_eq!(rest, vec!["--flag", "arg"]);
    }

    #[test]
    fn double_dash_with_empty_args_unchanged() {
        let args: Vec<String> = vec![];
        let (cmd, rest) = extract_args_after_double_dash("--", &args);
        assert_eq!(cmd, "--");
        assert!(rest.is_empty());
    }

    #[test]
    fn non_double_dash_command_unchanged() {
        let args = argv(&["x"]);
        let (cmd, rest) = extract_args_after_double_dash("name", &args);
        assert_eq!(cmd, "name");
        assert_eq!(rest, vec!["x"]);
    }
}
