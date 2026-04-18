//! Slash-command input tokeniser.
//!
//! Port of TS `src/utils/slashCommandParsing.ts`. Splits the raw
//! input line the user typed (`/cmd arg1 arg2`) into the command
//! name, the rest of the arguments as a single string, and a flag
//! indicating whether the second word was the `(MCP)` marker.
//! The `(MCP)` marker is appended to the command name so callers
//! can look it up verbatim in the MCP command registry.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSlashCommand {
    pub command_name: String,
    pub args: String,
    pub is_mcp: bool,
}

/// Parse a slash-command input line. Returns `None` when the input
/// doesn't begin with `/` or has no word after the slash.
pub fn parse_slash_command(input: &str) -> Option<ParsedSlashCommand> {
    let trimmed = input.trim();
    let without_slash = trimmed.strip_prefix('/')?;
    let mut words = without_slash.split(' ').peekable();
    let first = words.next()?;
    if first.is_empty() {
        return None;
    }

    let mut command_name = first.to_string();
    let mut is_mcp = false;
    if let Some(&second) = words.peek() {
        if second == "(MCP)" {
            command_name.push_str(" (MCP)");
            is_mcp = true;
            words.next();
        }
    }

    let args = words.collect::<Vec<_>>().join(" ");

    Some(ParsedSlashCommand {
        command_name,
        args,
        is_mcp,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_command() {
        let p = parse_slash_command("/search foo bar").unwrap();
        assert_eq!(p.command_name, "search");
        assert_eq!(p.args, "foo bar");
        assert!(!p.is_mcp);
    }

    #[test]
    fn parses_command_with_no_args() {
        let p = parse_slash_command("/clear").unwrap();
        assert_eq!(p.command_name, "clear");
        assert_eq!(p.args, "");
        assert!(!p.is_mcp);
    }

    #[test]
    fn parses_mcp_marker() {
        let p = parse_slash_command("/mcp:tool (MCP) arg1 arg2").unwrap();
        assert_eq!(p.command_name, "mcp:tool (MCP)");
        assert_eq!(p.args, "arg1 arg2");
        assert!(p.is_mcp);
    }

    #[test]
    fn mcp_marker_only_no_args() {
        let p = parse_slash_command("/mcp:ping (MCP)").unwrap();
        assert_eq!(p.command_name, "mcp:ping (MCP)");
        assert_eq!(p.args, "");
        assert!(p.is_mcp);
    }

    #[test]
    fn rejects_no_leading_slash() {
        assert!(parse_slash_command("search foo").is_none());
    }

    #[test]
    fn rejects_bare_slash() {
        assert!(parse_slash_command("/").is_none());
    }

    #[test]
    fn trims_surrounding_whitespace() {
        let p = parse_slash_command("   /help   ").unwrap();
        assert_eq!(p.command_name, "help");
    }

    #[test]
    fn preserves_arg_spacing_as_single_space() {
        // Internal runs of single spaces stay as-is; we don't
        // normalise multi-space runs to a single separator because
        // TS splits on literal " " and joins with " " — the second
        // arg here is "bar" with a leading empty-string token
        // collapsed when we re-join with " ".
        let p = parse_slash_command("/cmd foo  bar").unwrap();
        assert_eq!(p.args, "foo  bar");
    }

    #[test]
    fn mcp_marker_must_be_exact_parens() {
        // A word that merely contains "MCP" but isn't "(MCP)" must
        // not trigger the MCP branch.
        let p = parse_slash_command("/cmd MCP arg").unwrap();
        assert!(!p.is_mcp);
        assert_eq!(p.command_name, "cmd");
        assert_eq!(p.args, "MCP arg");
    }
}
