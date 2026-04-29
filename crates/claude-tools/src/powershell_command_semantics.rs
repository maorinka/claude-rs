//! PowerShell-specific exit-code interpretation.
//!
//! Port of `src/tools/PowerShellTool/commandSemantics.ts`. PowerShell-native
//! cmdlets mostly report failures through `$?`, but external executables still
//! use process exit codes, and some non-zero codes are informational.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interpretation {
    pub is_error: bool,
    pub message: Option<String>,
}

pub fn interpret_command_result(
    command: &str,
    exit_code: i32,
    _stdout: &str,
    _stderr: &str,
) -> Interpretation {
    match heuristically_extract_base_command(command).as_str() {
        "grep" | "rg" | "findstr" => Interpretation {
            is_error: exit_code >= 2,
            message: if exit_code == 1 {
                Some("No matches found".into())
            } else {
                None
            },
        },
        "robocopy" => Interpretation {
            is_error: exit_code >= 8,
            message: if exit_code == 0 {
                Some("No files copied (already in sync)".into())
            } else if (1..8).contains(&exit_code) {
                if exit_code & 1 == 1 {
                    Some("Files copied successfully".into())
                } else {
                    Some("Robocopy completed (no errors)".into())
                }
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

fn extract_base_command(segment: &str) -> String {
    let trimmed = segment.trim();
    let stripped = if matches!(trimmed.as_bytes().first(), Some(b'&' | b'.'))
        && trimmed[1..]
            .chars()
            .next()
            .map(char::is_whitespace)
            .unwrap_or(false)
    {
        trimmed[1..].trim_start()
    } else {
        trimmed
    };
    let first = stripped.split_whitespace().next().unwrap_or("");
    let unquoted = first.trim_matches('"').trim_matches('\'');
    let basename = unquoted
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(unquoted)
        .to_ascii_lowercase();
    basename
        .strip_suffix(".exe")
        .unwrap_or(&basename)
        .to_string()
}

fn heuristically_extract_base_command(command: &str) -> String {
    let last = command
        .split([';', '|'])
        .filter(|segment| !segment.trim().is_empty())
        .next_back()
        .unwrap_or(command);
    extract_base_command(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grep_and_ripgrep_no_match_are_not_errors() {
        for command in ["grep foo file.txt", "rg foo src", "findstr foo file.txt"] {
            let r = interpret_command_result(command, 1, "", "");
            assert!(!r.is_error, "{command}");
            assert_eq!(r.message.as_deref(), Some("No matches found"));
        }
    }

    #[test]
    fn grep_real_errors_are_errors() {
        let r = interpret_command_result("rg foo src", 2, "", "");
        assert!(r.is_error);
    }

    #[test]
    fn robocopy_success_bitfield_is_not_error() {
        let r = interpret_command_result("robocopy src dst", 3, "", "");
        assert!(!r.is_error);
        assert_eq!(r.message.as_deref(), Some("Files copied successfully"));
    }

    #[test]
    fn robocopy_error_bitfield_is_error() {
        let r = interpret_command_result("robocopy src dst", 8, "", "");
        assert!(r.is_error);
    }

    #[test]
    fn strips_call_operator_quotes_paths_and_exe_suffix() {
        let r = interpret_command_result(r#"& "C:\Tools\rg.exe" foo ."#, 1, "", "");
        assert!(!r.is_error);
        assert_eq!(r.message.as_deref(), Some("No matches found"));
    }

    #[test]
    fn last_pipeline_segment_determines_exit_code() {
        let r = interpret_command_result("Get-Content file.txt | rg foo", 1, "", "");
        assert!(!r.is_error);
    }

    #[test]
    fn default_semantics_flag_nonzero() {
        let r = interpret_command_result("custom.exe", 2, "", "");
        assert!(r.is_error);
        assert_eq!(
            r.message.as_deref(),
            Some("Command failed with exit code 2")
        );
    }
}
