//! Per-command exit-code interpretation.
//!
//! Port of `src/tools/BashTool/commandSemantics.ts`. Many commands use
//! exit codes to convey non-error information (grep 1 = no matches,
//! diff 1 = differences found, find 1 = partial success). Without
//! per-command semantics, BashTool surfaces a misleading "Command failed"
//! when the command actually did what it was asked.

/// Outcome of interpreting a command's exit code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interpretation {
    pub is_error: bool,
    pub message: Option<String>,
}

/// Interpret a command's exit code + stdout/stderr under the
/// command-specific semantics. Unknown commands fall back to
/// "anything non-zero is an error".
pub fn interpret_command_result(
    command: &str,
    exit_code: i32,
    _stdout: &str,
    _stderr: &str,
) -> Interpretation {
    let base = heuristically_extract_base_command(command);
    match base {
        "grep" | "rg" => Interpretation {
            is_error: exit_code >= 2,
            message: if exit_code == 1 {
                Some("No matches found".into())
            } else {
                None
            },
        },
        "find" => Interpretation {
            is_error: exit_code >= 2,
            message: if exit_code == 1 {
                Some("Some directories were inaccessible".into())
            } else {
                None
            },
        },
        "diff" => Interpretation {
            is_error: exit_code >= 2,
            message: if exit_code == 1 {
                Some("Files differ".into())
            } else {
                None
            },
        },
        "test" | "[" => Interpretation {
            is_error: exit_code >= 2,
            message: if exit_code == 1 {
                Some("Condition is false".into())
            } else {
                None
            },
        },
        _ => Interpretation {
            is_error: exit_code != 0,
            message: if exit_code != 0 {
                Some(format!("Command failed with exit code {}", exit_code))
            } else {
                None
            },
        },
    }
}

fn extract_base_command(command: &str) -> &str {
    command
        .trim()
        .split_whitespace()
        .next()
        .unwrap_or("")
}

/// Cheap split on `;`, `&&`, `||`, `|` respecting single/double quotes.
/// Returns the segments in order.
fn split_command(command: &str) -> Vec<&str> {
    let bytes = command.as_bytes();
    let mut out = Vec::new();
    let mut start = 0;
    let mut in_single = false;
    let mut in_double = false;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        match c {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '|' | '&' | ';' if !in_single && !in_double => {
                // Consume doubled operator (||, &&) atomically.
                let step = if (c == '|' || c == '&')
                    && i + 1 < bytes.len()
                    && bytes[i + 1] as char == c
                {
                    2
                } else {
                    1
                };
                let seg = command[start..i].trim();
                if !seg.is_empty() {
                    out.push(seg);
                }
                i += step;
                start = i;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    let tail = command[start..].trim();
    if !tail.is_empty() {
        out.push(tail);
    }
    out
}

/// Heuristically extract the primary command — the last segment, since
/// pipeline exit codes are determined by the final command. Matches TS
/// `heuristicallyExtractBaseCommand`.
fn heuristically_extract_base_command(command: &str) -> &str {
    let segments = split_command(command);
    let last = segments.last().copied().unwrap_or(command);
    extract_base_command(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grep_no_match_is_not_error() {
        let r = interpret_command_result("grep foo bar.txt", 1, "", "");
        assert!(!r.is_error);
        assert_eq!(r.message.as_deref(), Some("No matches found"));
    }

    #[test]
    fn grep_real_error_is_error() {
        let r = interpret_command_result("grep foo bar.txt", 2, "", "");
        assert!(r.is_error);
    }

    #[test]
    fn diff_differences_found_is_not_error() {
        let r = interpret_command_result("diff a b", 1, "", "");
        assert!(!r.is_error);
        assert_eq!(r.message.as_deref(), Some("Files differ"));
    }

    #[test]
    fn test_condition_false_is_not_error() {
        let r = interpret_command_result("test -f missing.txt", 1, "", "");
        assert!(!r.is_error);
        assert_eq!(r.message.as_deref(), Some("Condition is false"));
    }

    #[test]
    fn default_semantic_flags_nonzero() {
        let r = interpret_command_result("ls /nope", 2, "", "");
        assert!(r.is_error);
        assert!(r
            .message
            .as_deref()
            .unwrap_or("")
            .contains("exit code 2"));
    }

    #[test]
    fn pipeline_last_command_wins() {
        // "ls | grep foo" → exit code comes from grep → 1 means no matches
        let r = interpret_command_result("ls /tmp | grep foo", 1, "", "");
        assert!(!r.is_error);
    }

    #[test]
    fn chained_and_takes_last() {
        // "make && test -f out" → exit code from test
        let r = interpret_command_result("make && test -f out", 1, "", "");
        assert!(!r.is_error);
    }

    #[test]
    fn quoted_semicolon_not_a_separator() {
        // `echo 'a;b'` is one command, not two.
        let r = interpret_command_result("echo 'a;b'", 0, "", "");
        assert!(!r.is_error);
    }

    #[test]
    fn ripgrep_has_grep_semantics() {
        let r = interpret_command_result("rg pattern src/", 1, "", "");
        assert!(!r.is_error);
        assert_eq!(r.message.as_deref(), Some("No matches found"));
    }
}
