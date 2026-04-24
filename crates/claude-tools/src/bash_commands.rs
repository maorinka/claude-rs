//! Light-weight bash command helpers.
//!
//! TS `src/utils/bash/commands.ts` is 1,339 LOC covering a security-
//! critical splitter built on shell-quote + tree-sitter with heredoc
//! extraction, placeholder-injection defences, path-constraint hooks,
//! and a compound-unsafety classifier. Porting that faithfully is its
//! own project.
//!
//! This module ports the non-tree-sitter helpers that callers need
//! today — **all with the explicit caveat that security-sensitive
//! decisions should NOT rely on this module alone**. Treat the output
//! as best-effort classification and layer real validation
//! (sed_validation, destructive_command_warning, the coarse
//! read-only classifier) on top.
//!
//! Ported:
//!   - `extract_bash_comment_label` — full 13-line port, non-security
//!   - `is_help_command` — `cmd --help` with no other suspicious flags
//!   - `extract_output_redirections` — cheap `> FILE` / `>> FILE`
//!     scan that respects single/double quotes

/// If the first line of a bash command is a `# comment` (not a `#!`
/// shebang), return the stripped comment text. Used by the TUI as the
/// tool-use label shown to the user.
///
/// Full port of TS `extractBashCommentLabel`.
pub fn extract_bash_comment_label(command: &str) -> Option<String> {
    let first_line = command.split('\n').next().unwrap_or(command).trim();
    if !first_line.starts_with('#') || first_line.starts_with("#!") {
        return None;
    }
    let stripped = first_line.trim_start_matches('#').trim_start();
    if stripped.is_empty() {
        None
    } else {
        Some(stripped.to_string())
    }
}

/// Is this a simple `--help` invocation? Accepts only commands where
/// every non-flag token is alphanumeric and no flag other than `--help`
/// appears — rejects anything with quotes, special chars, or paths.
///
/// Port of TS `isHelpCommand` without the shell-quote dependency.
pub fn is_help_command(command: &str) -> bool {
    let trimmed = command.trim();
    if !trimmed.ends_with("--help") {
        return false;
    }
    // Reject quoted variants that might be trying to bypass the check.
    if trimmed.contains('"') || trimmed.contains('\'') {
        return false;
    }
    let mut found_help = false;
    for tok in trimmed.split_whitespace() {
        if tok.starts_with('-') {
            if tok == "--help" {
                found_help = true;
            } else {
                return false;
            }
        } else if !tok.chars().all(|c| c.is_ascii_alphanumeric()) {
            return false;
        }
    }
    found_help
}

/// Redirection targets found in the command along with their operator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redirection {
    pub target: String,
    pub operator: RedirOp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedirOp {
    Overwrite,
    Append,
}

/// Extract `> TARGET` and `>> TARGET` redirection sites. Respects
/// single + double quote contexts so `echo 'text > file.txt'` is NOT
/// flagged. Process substitution (`>(cmd)`), file descriptors (`2>&1`,
/// `>&2`), and heredocs are NOT handled — callers that need
/// heredoc-safe extraction should layer on a real tokeniser.
///
/// Cheap-and-right for the common case, which is all we need to
/// surface "this command writes to disk" warnings.
pub fn extract_output_redirections(command: &str) -> Vec<Redirection> {
    let bytes = command.as_bytes();
    let mut out = Vec::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut i = 0;

    while i < bytes.len() {
        let c = bytes[i] as char;
        match c {
            '\\' if !in_single => {
                // Skip the next char — it's escaped.
                i += 2;
                continue;
            }
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '>' if !in_single && !in_double => {
                // Ignore `2>`, `&>`, `>&`, `>(` (process sub).
                let prev = if i > 0 {
                    Some(bytes[i - 1] as char)
                } else {
                    None
                };
                if matches!(prev, Some('0'..='9') | Some('&')) {
                    i += 1;
                    continue;
                }
                let (op, skip) = if i + 1 < bytes.len() && bytes[i + 1] as char == '>' {
                    (RedirOp::Append, 2)
                } else if i + 1 < bytes.len() && bytes[i + 1] as char == '&' {
                    // FD redirect like `>&2` — skip.
                    i += 2;
                    continue;
                } else if i + 1 < bytes.len() && bytes[i + 1] as char == '(' {
                    // Process substitution — skip.
                    i += 2;
                    continue;
                } else {
                    (RedirOp::Overwrite, 1)
                };
                i += skip;
                // Skip whitespace between operator and target.
                while i < bytes.len() && (bytes[i] as char).is_whitespace() {
                    i += 1;
                }
                // Consume target until whitespace / operator / end.
                let target_start = i;
                while i < bytes.len() {
                    let t = bytes[i] as char;
                    if t.is_whitespace() || t == ';' || t == '|' || t == '&' {
                        break;
                    }
                    i += 1;
                }
                if i > target_start {
                    let target = command[target_start..i].to_string();
                    if !target.is_empty() {
                        out.push(Redirection {
                            target,
                            operator: op,
                        });
                    }
                }
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comment_label_extracts() {
        assert_eq!(
            extract_bash_comment_label("# install deps\nnpm install"),
            Some("install deps".into())
        );
    }

    #[test]
    fn comment_label_ignores_shebang() {
        assert_eq!(
            extract_bash_comment_label("#!/usr/bin/env bash\necho hi"),
            None
        );
    }

    #[test]
    fn comment_label_none_when_no_comment() {
        assert!(extract_bash_comment_label("ls -la").is_none());
    }

    #[test]
    fn comment_label_strips_multiple_hashes_and_ws() {
        assert_eq!(
            extract_bash_comment_label("### header"),
            Some("header".into())
        );
    }

    #[test]
    fn help_command_accepted() {
        assert!(is_help_command("ls --help"));
        assert!(is_help_command("git --help"));
    }

    #[test]
    fn help_command_rejects_other_flags() {
        assert!(!is_help_command("ls -la --help"));
        assert!(!is_help_command("cat /etc/passwd --help"));
    }

    #[test]
    fn help_command_rejects_quotes() {
        assert!(!is_help_command("ls \"--help\""));
        assert!(!is_help_command("'ls' --help"));
    }

    #[test]
    fn extract_redirection_basic() {
        let r = extract_output_redirections("echo hi > out.txt");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].target, "out.txt");
        assert_eq!(r[0].operator, RedirOp::Overwrite);
    }

    #[test]
    fn extract_redirection_append() {
        let r = extract_output_redirections("echo hi >> log");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].target, "log");
        assert_eq!(r[0].operator, RedirOp::Append);
    }

    #[test]
    fn quoted_redirect_is_ignored() {
        let r = extract_output_redirections("echo 'text > file.txt'");
        assert!(r.is_empty());
    }

    #[test]
    fn fd_redirect_ignored() {
        let r = extract_output_redirections("cmd 2>&1");
        assert!(r.is_empty());
    }

    #[test]
    fn multiple_redirections_captured() {
        let r = extract_output_redirections("cmd > a.txt; other >> b.log");
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].target, "a.txt");
        assert_eq!(r[1].target, "b.log");
    }

    #[test]
    fn escaped_gt_ignored() {
        // Backslash-escaped > is not a redirection.
        let r = extract_output_redirections(r#"echo hi \> not-a-redir"#);
        assert!(r.is_empty());
    }
}
