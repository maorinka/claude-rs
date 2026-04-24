//! Bash AST security parser -- destructive-command detection before execution.
//!
//! Port of the TS bash security system from:
//!   src/tools/BashTool/bashSecurity.ts
//!   src/tools/BashTool/bashPermissions.ts
//!   src/utils/bash/ast.ts
//!
//! Key design: FAIL-CLOSED. Any pattern we don't explicitly recognize as safe
//! goes to "ask" (user prompt). This is NOT a sandbox -- it answers: "Can we
//! prove this command is safe without asking the user?"

// ---------------------------------------------------------------------------
// Security validation types
// ---------------------------------------------------------------------------

/// The result of a security validation check on a bash command.
#[derive(Debug, Clone, PartialEq)]
pub enum SecurityBehavior {
    /// Command passed this check (continue to next validator).
    Passthrough,
    /// Command is explicitly allowed by this check.
    Allow,
    /// Command needs user confirmation.
    Ask(String),
    /// Command is denied.
    #[allow(dead_code)]
    Deny(String),
}

// ---------------------------------------------------------------------------
// Pre-parse security checks (runs on raw command text before splitting)
// ---------------------------------------------------------------------------

/// Control characters that bash silently drops but confuse static analysis.
pub fn has_control_chars(cmd: &str) -> bool {
    cmd.bytes()
        .any(|b| matches!(b, 0x00..=0x08 | 0x0B..=0x1F | 0x7F))
}

/// Unicode whitespace beyond ASCII. Blocks NBSP, zero-width spaces, BOM, etc.
pub fn has_unicode_whitespace(cmd: &str) -> bool {
    cmd.chars().any(|c| {
        matches!(
            c,
            '\u{00A0}' | '\u{1680}' | '\u{2000}'
                ..='\u{200B}'
                    | '\u{2028}'
                    | '\u{2029}'
                    | '\u{202F}'
                    | '\u{205F}'
                    | '\u{3000}'
                    | '\u{FEFF}'
        )
    })
}

/// Backslash immediately before whitespace outside quotes.
pub fn has_backslash_whitespace(cmd: &str) -> bool {
    let bytes = cmd.as_bytes();
    let len = bytes.len();
    for i in 0..len {
        if bytes[i] == b'\\' && i + 1 < len && (bytes[i + 1] == b' ' || bytes[i + 1] == b'\t') {
            return true;
        }
        if i + 2 < len
            && !matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\\')
            && bytes[i + 1] == b'\\'
            && bytes[i + 2] == b'\n'
        {
            return true;
        }
    }
    false
}

/// Zsh dynamic named directory expansion: ~[name].
pub fn has_zsh_tilde_bracket(cmd: &str) -> bool {
    cmd.contains("~[")
}

/// Zsh EQUALS expansion: word-initial `=cmd` expands to the absolute path.
pub fn has_zsh_equals_expansion(cmd: &str) -> bool {
    let bytes = cmd.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'=' {
            let at_word_start = i == 0 || matches!(bytes[i - 1], b' ' | b'\t' | b';' | b'&' | b'|');
            if at_word_start
                && i + 1 < bytes.len()
                && (bytes[i + 1].is_ascii_alphabetic() || bytes[i + 1] == b'_')
            {
                return true;
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Quote-aware content extraction -- matches TS `extractQuotedContent`
// ---------------------------------------------------------------------------

struct QuoteExtraction {
    with_double_quotes: String,
    fully_unquoted: String,
    unquoted_keep_quote_chars: String,
}

fn extract_quoted_content(command: &str) -> QuoteExtraction {
    let mut with_double_quotes = String::new();
    let mut fully_unquoted = String::new();
    let mut unquoted_keep_quote_chars = String::new();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut escaped = false;

    for ch in command.chars() {
        if escaped {
            escaped = false;
            if !in_single_quote {
                with_double_quotes.push(ch);
            }
            if !in_single_quote && !in_double_quote {
                fully_unquoted.push(ch);
                unquoted_keep_quote_chars.push(ch);
            }
            continue;
        }
        if ch == '\\' && !in_single_quote {
            escaped = true;
            if !in_single_quote {
                with_double_quotes.push(ch);
            }
            if !in_double_quote {
                fully_unquoted.push(ch);
                unquoted_keep_quote_chars.push(ch);
            }
            continue;
        }
        if ch == '\'' && !in_double_quote {
            in_single_quote = !in_single_quote;
            unquoted_keep_quote_chars.push(ch);
            continue;
        }
        if ch == '"' && !in_single_quote {
            in_double_quote = !in_double_quote;
            unquoted_keep_quote_chars.push(ch);
            continue;
        }
        if !in_single_quote {
            with_double_quotes.push(ch);
        }
        if !in_single_quote && !in_double_quote {
            fully_unquoted.push(ch);
            unquoted_keep_quote_chars.push(ch);
        }
    }

    QuoteExtraction {
        with_double_quotes,
        fully_unquoted,
        unquoted_keep_quote_chars,
    }
}

fn strip_safe_redirections(content: &str) -> String {
    let r1 = regex_lite::Regex::new(r"\s+2\s*>&\s*1(?:\s|$)").unwrap();
    let r2 = regex_lite::Regex::new(r"[012]?\s*>\s*/dev/null(?:\s|$)").unwrap();
    let r3 = regex_lite::Regex::new(r"\s*<\s*/dev/null(?:\s|$)").unwrap();
    let result = r1.replace_all(content, " ");
    let result = r2.replace_all(&result, " ");
    r3.replace_all(&result, " ").into_owned()
}

// ---------------------------------------------------------------------------
// Validation context (matches TS ValidationContext)
// ---------------------------------------------------------------------------

struct ValidationContext {
    original_command: String,
    base_command: String,
    unquoted_content: String,
    fully_unquoted_content: String,
    fully_unquoted_pre_strip: String,
    unquoted_keep_quote_chars: String,
}

impl ValidationContext {
    fn new(command: &str) -> Self {
        let base_command = command.split_whitespace().next().unwrap_or("").to_string();
        let extraction = extract_quoted_content(command);
        let fully_unquoted_stripped = strip_safe_redirections(&extraction.fully_unquoted);
        ValidationContext {
            original_command: command.to_string(),
            base_command,
            unquoted_content: extraction.with_double_quotes,
            fully_unquoted_content: fully_unquoted_stripped,
            fully_unquoted_pre_strip: extraction.fully_unquoted,
            unquoted_keep_quote_chars: extraction.unquoted_keep_quote_chars,
        }
    }
}

// ---------------------------------------------------------------------------
// Individual security validators
// ---------------------------------------------------------------------------

fn validate_empty(ctx: &ValidationContext) -> SecurityBehavior {
    if ctx.original_command.trim().is_empty() {
        return SecurityBehavior::Allow;
    }
    SecurityBehavior::Passthrough
}

fn validate_incomplete_commands(ctx: &ValidationContext) -> SecurityBehavior {
    let trimmed = ctx.original_command.trim();
    if ctx.original_command.starts_with('\t') || ctx.original_command.starts_with(" \t") {
        return SecurityBehavior::Ask(
            "Command appears to be an incomplete fragment (starts with tab)".into(),
        );
    }
    if trimmed.starts_with('-') {
        return SecurityBehavior::Ask(
            "Command appears to be an incomplete fragment (starts with flags)".into(),
        );
    }
    if trimmed.starts_with("&&")
        || trimmed.starts_with("||")
        || trimmed.starts_with(';')
        || trimmed.starts_with(">>")
        || trimmed.starts_with('>')
        || trimmed.starts_with('<')
    {
        return SecurityBehavior::Ask(
            "Command appears to be a continuation line (starts with operator)".into(),
        );
    }
    SecurityBehavior::Passthrough
}

/// Detect safe `git commit -m "message"` pattern and allow it early.
/// Matches TS `validateGitCommit` from bashSecurity.ts.
fn validate_git_commit(ctx: &ValidationContext) -> SecurityBehavior {
    let cmd = &ctx.original_command;
    if ctx.base_command != "git"
        || !regex_lite::Regex::new(r"^git\s+commit\s+")
            .unwrap()
            .is_match(cmd)
    {
        return SecurityBehavior::Passthrough;
    }
    // Bail on backslashes (can desync quote detection)
    if cmd.contains('\\') {
        return SecurityBehavior::Passthrough;
    }
    // Find -m flag followed by quoted message. regex_lite does not support
    // backreferences, so we manually find the quote char and its match.
    let before_m =
        regex_lite::Regex::new(r"^git[ \t]+commit[ \t]+[^;&|`$<>()\n\r]*?-m[ \t]+").unwrap();
    let m = match before_m.find(cmd) {
        Some(m) => m,
        None => return SecurityBehavior::Passthrough,
    };
    let after = &cmd[m.end()..];
    if after.is_empty() {
        return SecurityBehavior::Passthrough;
    }
    let quote_char = after.chars().next().unwrap();
    if quote_char != '\'' && quote_char != '"' {
        return SecurityBehavior::Passthrough;
    }
    // Find the matching closing quote
    let inner = &after[1..];
    let close_pos = match inner.find(quote_char) {
        Some(p) => p,
        None => return SecurityBehavior::Passthrough,
    };
    let msg_content = &inner[..close_pos];
    let remainder = &inner[close_pos + 1..]; // after closing quote

    // Double-quoted message with command substitution => ask
    if quote_char == '"'
        && regex_lite::Regex::new(r"\$\(|`|\$\{")
            .unwrap()
            .is_match(msg_content)
    {
        return SecurityBehavior::Ask(
            "Git commit message contains command substitution patterns".into(),
        );
    }
    // Shell metacharacters in remainder => passthrough for full validation
    if !remainder.is_empty()
        && regex_lite::Regex::new(r"[;|&()`]|\$\(|\$\{")
            .unwrap()
            .is_match(remainder)
    {
        return SecurityBehavior::Passthrough;
    }
    // Unquoted redirect operators in remainder => passthrough
    if !remainder.is_empty() {
        let mut unquoted = String::new();
        let mut in_sq = false;
        let mut in_dq = false;
        for c in remainder.chars() {
            if c == '\'' && !in_dq {
                in_sq = !in_sq;
                continue;
            }
            if c == '"' && !in_sq {
                in_dq = !in_dq;
                continue;
            }
            if !in_sq && !in_dq {
                unquoted.push(c);
            }
        }
        if unquoted.contains('<') || unquoted.contains('>') {
            return SecurityBehavior::Passthrough;
        }
    }
    // Message starting with dash => obfuscation
    if msg_content.starts_with('-') {
        return SecurityBehavior::Ask("Command contains quoted characters in flag names".into());
    }
    SecurityBehavior::Allow
}

/// Detect safe heredoc-in-substitution patterns: $(cat <<'EOF'...EOF)
/// Matches TS `validateSafeCommandSubstitution` from bashSecurity.ts.
fn validate_safe_command_substitution(ctx: &ValidationContext) -> SecurityBehavior {
    let cmd = &ctx.original_command;
    // Only applies to commands containing heredoc-in-substitution
    if !regex_lite::Regex::new(r"\$\(.*<<").unwrap().is_match(cmd) {
        return SecurityBehavior::Passthrough;
    }
    // Check for quoted/escaped delimiter pattern: $(cat <<'DELIM' or <<\DELIM
    let re =
        regex_lite::Regex::new(r"\$\(cat[ \t]*<<-?[ \t]*(?:'+([A-Za-z_]\w*)'+|\\([A-Za-z_]\w*))")
            .unwrap();
    if !re.is_match(cmd) {
        return SecurityBehavior::Passthrough;
    }
    // We found a safe heredoc pattern. For safety, just let it pass through
    // to the full validator chain (fail-closed approach). The TS does more
    // sophisticated line-based heredoc body stripping, which requires the
    // tree-sitter parser. Without that, we conservatively passthrough.
    SecurityBehavior::Passthrough
}

/// Detect malformed tokens (unbalanced delimiters) with command separators.
/// Matches TS `validateMalformedTokenInjection` from bashSecurity.ts.
fn validate_malformed_token_injection(ctx: &ValidationContext) -> SecurityBehavior {
    let cmd = &ctx.original_command;
    // Quick check: does the command contain separators?
    let has_separator = cmd.contains(';') || cmd.contains("&&") || cmd.contains("||");
    if !has_separator {
        return SecurityBehavior::Passthrough;
    }
    // Check for unbalanced delimiters: count quotes, parens, braces, brackets
    let mut sq_count = 0u32;
    let mut dq_count = 0u32;
    let mut paren_depth: i32 = 0;
    let mut brace_depth: i32 = 0;
    let mut bracket_depth: i32 = 0;
    let mut escaped = false;
    for ch in cmd.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        match ch {
            '\'' => sq_count += 1,
            '"' => dq_count += 1,
            '(' => paren_depth += 1,
            ')' => paren_depth -= 1,
            '{' => brace_depth += 1,
            '}' => brace_depth -= 1,
            '[' => bracket_depth += 1,
            ']' => bracket_depth -= 1,
            _ => {},
        }
    }
    // Odd quote counts or unbalanced delimiters with separators is suspicious
    if (!sq_count.is_multiple_of(2)
        || !dq_count.is_multiple_of(2)
        || paren_depth != 0
        || brace_depth != 0
        || bracket_depth != 0)
        && has_separator
    {
        return SecurityBehavior::Ask(
            "Command contains ambiguous syntax with command separators".into(),
        );
    }
    SecurityBehavior::Passthrough
}

fn validate_jq_command(ctx: &ValidationContext) -> SecurityBehavior {
    if ctx.base_command != "jq" {
        return SecurityBehavior::Passthrough;
    }
    if ctx.original_command.contains("system") && ctx.original_command.contains('(') {
        return SecurityBehavior::Ask(
            "jq command contains system() function which executes arbitrary commands".into(),
        );
    }
    let after_jq = if ctx.original_command.len() > 3 {
        ctx.original_command[3..].trim()
    } else {
        ""
    };
    for flag in &[
        "-f",
        "--from-file",
        "--rawfile",
        "--slurpfile",
        "-L",
        "--library-path",
    ] {
        if after_jq.contains(flag) {
            return SecurityBehavior::Ask(
                "jq command contains dangerous flags that could read arbitrary files".into(),
            );
        }
    }
    SecurityBehavior::Passthrough
}

fn validate_obfuscated_flags(ctx: &ValidationContext) -> SecurityBehavior {
    let cmd = &ctx.original_command;
    if ctx.base_command == "echo" && !cmd.contains('|') && !cmd.contains('&') && !cmd.contains(';')
    {
        return SecurityBehavior::Passthrough;
    }
    if regex_lite::Regex::new(r"\$'[^']*'").unwrap().is_match(cmd) {
        return SecurityBehavior::Ask(
            "Command contains ANSI-C quoting which can hide characters".into(),
        );
    }
    if regex_lite::Regex::new(r#"\$"[^"]*""#)
        .unwrap()
        .is_match(cmd)
    {
        return SecurityBehavior::Ask(
            "Command contains locale quoting which can hide characters".into(),
        );
    }
    if regex_lite::Regex::new(r#"\$['"]['"]\s*-"#)
        .unwrap()
        .is_match(cmd)
    {
        return SecurityBehavior::Ask("Command contains empty special quotes before dash".into());
    }
    if regex_lite::Regex::new(r#"(?:^|\s)(?:''|"")+\s*-"#)
        .unwrap()
        .is_match(cmd)
    {
        return SecurityBehavior::Ask("Command contains empty quotes before dash".into());
    }
    if regex_lite::Regex::new(r#"(?:""|'')+['"]-"#)
        .unwrap()
        .is_match(cmd)
    {
        return SecurityBehavior::Ask(
            "Command contains empty quote pair adjacent to quoted dash".into(),
        );
    }
    if regex_lite::Regex::new(r#"(?:^|\s)['\"]{3,}"#)
        .unwrap()
        .is_match(cmd)
    {
        return SecurityBehavior::Ask(
            "Command contains consecutive quote characters at word start".into(),
        );
    }
    if regex_lite::Regex::new(r#"\s['"`]-"#)
        .unwrap()
        .is_match(&ctx.fully_unquoted_content)
    {
        return SecurityBehavior::Ask("Command contains quoted characters in flag names".into());
    }
    if regex_lite::Regex::new(r#"['"`]{2}-"#)
        .unwrap()
        .is_match(&ctx.fully_unquoted_content)
    {
        return SecurityBehavior::Ask("Command contains quoted characters in flag names".into());
    }
    SecurityBehavior::Passthrough
}

fn validate_shell_metacharacters(ctx: &ValidationContext) -> SecurityBehavior {
    let content = &ctx.unquoted_content;
    if regex_lite::Regex::new(r#"(?:^|\s)["'][^"']*[;&][^"']*["'](?:\s|$)"#)
        .unwrap()
        .is_match(content)
    {
        return SecurityBehavior::Ask("Command contains shell metacharacters in arguments".into());
    }
    for pat in &[
        r#"-name\s+["'][^"']*[;|&][^"']*["']"#,
        r#"-path\s+["'][^"']*[;|&][^"']*["']"#,
        r#"-iname\s+["'][^"']*[;|&][^"']*["']"#,
    ] {
        if regex_lite::Regex::new(pat).unwrap().is_match(content) {
            return SecurityBehavior::Ask(
                "Command contains shell metacharacters in arguments".into(),
            );
        }
    }
    SecurityBehavior::Passthrough
}

fn validate_dangerous_variables(ctx: &ValidationContext) -> SecurityBehavior {
    let content = &ctx.fully_unquoted_content;
    if regex_lite::Regex::new(r"[<>|]\s*\$[A-Za-z_]")
        .unwrap()
        .is_match(content)
        || regex_lite::Regex::new(r"\$[A-Za-z_][A-Za-z0-9_]*\s*[|<>]")
            .unwrap()
            .is_match(content)
    {
        return SecurityBehavior::Ask("Command contains variables in dangerous contexts".into());
    }
    SecurityBehavior::Passthrough
}

fn has_unescaped_char(content: &str, target: char) -> bool {
    let bytes = content.as_bytes();
    let tb = target as u8;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 2;
            continue;
        }
        if bytes[i] == tb {
            return true;
        }
        i += 1;
    }
    false
}

fn validate_dangerous_patterns(ctx: &ValidationContext) -> SecurityBehavior {
    let content = &ctx.unquoted_content;
    if has_unescaped_char(content, '`') {
        return SecurityBehavior::Ask("Command contains backticks for command substitution".into());
    }
    let patterns: &[(&str, &str)] = &[
        (r"<\(", "process substitution <()"),
        (r">\(", "process substitution >()"),
        (r"=\(", "Zsh process substitution =()"),
        (r"(?:^|[\s;&|])=[a-zA-Z_]", "Zsh equals expansion"),
        (r"\$\(", "$() command substitution"),
        (r"\$\{", "${} parameter substitution"),
        (r"\$\[", "$[] legacy arithmetic expansion"),
        (r"~\[", "Zsh-style parameter expansion"),
        (r"\(e:", "Zsh-style glob qualifiers"),
        (r"\(\+", "Zsh glob qualifier with command execution"),
        (r"\}\s*always\s*\{", "Zsh always block"),
        (r"<#", "PowerShell comment syntax"),
    ];
    for (pat, msg) in patterns {
        if regex_lite::Regex::new(pat).unwrap().is_match(content) {
            return SecurityBehavior::Ask(format!("Command contains {}", msg));
        }
    }
    SecurityBehavior::Passthrough
}

fn validate_redirections(ctx: &ValidationContext) -> SecurityBehavior {
    let content = &ctx.fully_unquoted_content;
    if content.contains('<') {
        return SecurityBehavior::Ask("Command contains input redirection (<)".into());
    }
    if content.contains('>') {
        return SecurityBehavior::Ask("Command contains output redirection (>)".into());
    }
    SecurityBehavior::Passthrough
}

fn validate_newlines(ctx: &ValidationContext) -> SecurityBehavior {
    let content = &ctx.fully_unquoted_pre_strip;
    if !content.contains('\n') && !content.contains('\r') {
        return SecurityBehavior::Passthrough;
    }
    let bytes = content.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'\n' || bytes[i] == b'\r' {
            if i > 0 && bytes[i - 1] == b'\\' && i >= 2 && matches!(bytes[i - 2], b' ' | b'\t') {
                continue;
            }
            let mut j = i + 1;
            while j < bytes.len() && matches!(bytes[j], b' ' | b'\t') {
                j += 1;
            }
            if j < bytes.len() && !matches!(bytes[j], b'\n' | b'\r') {
                return SecurityBehavior::Ask(
                    "Command contains newlines that could separate multiple commands".into(),
                );
            }
        }
    }
    SecurityBehavior::Passthrough
}

fn validate_carriage_return(ctx: &ValidationContext) -> SecurityBehavior {
    let cmd = &ctx.original_command;
    if !cmd.contains('\r') {
        return SecurityBehavior::Passthrough;
    }
    let mut in_sq = false;
    let mut in_dq = false;
    let mut escaped = false;
    for ch in cmd.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' && !in_sq {
            escaped = true;
            continue;
        }
        if ch == '\'' && !in_dq {
            in_sq = !in_sq;
            continue;
        }
        if ch == '"' && !in_sq {
            in_dq = !in_dq;
            continue;
        }
        if ch == '\r' && !in_dq {
            return SecurityBehavior::Ask(
                "Command contains carriage return causing tokenization differential".into(),
            );
        }
    }
    SecurityBehavior::Passthrough
}

fn validate_ifs_injection(ctx: &ValidationContext) -> SecurityBehavior {
    let cmd = &ctx.original_command;
    if cmd.contains("$IFS")
        || regex_lite::Regex::new(r"\$\{[^}]*IFS")
            .unwrap()
            .is_match(cmd)
    {
        return SecurityBehavior::Ask("Command contains IFS variable usage".into());
    }
    SecurityBehavior::Passthrough
}

fn validate_proc_environ_access(ctx: &ValidationContext) -> SecurityBehavior {
    if regex_lite::Regex::new(r"/proc/.*/environ")
        .unwrap()
        .is_match(&ctx.original_command)
    {
        return SecurityBehavior::Ask("Command accesses /proc/*/environ".into());
    }
    SecurityBehavior::Passthrough
}

fn validate_backslash_escaped_whitespace(ctx: &ValidationContext) -> SecurityBehavior {
    let cmd = &ctx.original_command;
    let mut in_sq = false;
    let mut in_dq = false;
    let bytes = cmd.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && !in_sq {
            if !in_dq && i + 1 < bytes.len() && matches!(bytes[i + 1], b' ' | b'\t') {
                return SecurityBehavior::Ask(
                    "Command contains backslash-escaped whitespace".into(),
                );
            }
            i += 2;
            continue;
        }
        if bytes[i] == b'"' && !in_sq {
            in_dq = !in_dq;
        } else if bytes[i] == b'\'' && !in_dq {
            in_sq = !in_sq;
        }
        i += 1;
    }
    SecurityBehavior::Passthrough
}

fn validate_backslash_escaped_operators(ctx: &ValidationContext) -> SecurityBehavior {
    let cmd = &ctx.original_command;
    let mut in_sq = false;
    let mut in_dq = false;
    let bytes = cmd.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && !in_sq {
            if !in_dq
                && i + 1 < bytes.len()
                && matches!(bytes[i + 1], b';' | b'|' | b'&' | b'<' | b'>')
            {
                return SecurityBehavior::Ask(
                    "Command contains backslash before a shell operator".into(),
                );
            }
            i += 2;
            continue;
        }
        if bytes[i] == b'\'' && !in_dq {
            in_sq = !in_sq;
        } else if bytes[i] == b'"' && !in_sq {
            in_dq = !in_dq;
        }
        i += 1;
    }
    SecurityBehavior::Passthrough
}

fn validate_unicode_whitespace(ctx: &ValidationContext) -> SecurityBehavior {
    if has_unicode_whitespace(&ctx.original_command) {
        return SecurityBehavior::Ask("Command contains Unicode whitespace characters".into());
    }
    SecurityBehavior::Passthrough
}

fn validate_mid_word_hash(ctx: &ValidationContext) -> SecurityBehavior {
    let bytes = ctx.unquoted_keep_quote_chars.as_bytes();
    for i in 1..bytes.len() {
        if bytes[i] == b'#' && !matches!(bytes[i - 1], b' ' | b'\t' | b'\n' | b'\r') {
            if i >= 2 && bytes[i - 2] == b'$' && bytes[i - 1] == b'{' {
                continue;
            }
            return SecurityBehavior::Ask(
                "Command contains mid-word # (parser differential)".into(),
            );
        }
    }
    SecurityBehavior::Passthrough
}

fn is_escaped_at_position(bytes: &[u8], pos: usize) -> bool {
    let mut count = 0u32;
    let mut i = pos;
    while i > 0 {
        i -= 1;
        if bytes[i] == b'\\' {
            count += 1;
        } else {
            break;
        }
    }
    count % 2 == 1
}

fn validate_brace_expansion(ctx: &ValidationContext) -> SecurityBehavior {
    let content = &ctx.fully_unquoted_pre_strip;
    let bytes = content.as_bytes();
    let (mut open, mut close) = (0u32, 0u32);
    for i in 0..bytes.len() {
        if bytes[i] == b'{' && !is_escaped_at_position(bytes, i) {
            open += 1;
        } else if bytes[i] == b'}' && !is_escaped_at_position(bytes, i) {
            close += 1;
        }
    }
    if open > 0 && close > open {
        return SecurityBehavior::Ask(
            "Command has excess closing braces (brace expansion obfuscation)".into(),
        );
    }
    if open > 0
        && regex_lite::Regex::new(r#"['"][{}]['"]"#)
            .unwrap()
            .is_match(&ctx.original_command)
    {
        return SecurityBehavior::Ask("Command contains quoted brace inside brace context".into());
    }
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'{' || is_escaped_at_position(bytes, i) {
            i += 1;
            continue;
        }
        let mut depth: i32 = 1;
        let mut mc: Option<usize> = None;
        let mut j = i + 1;
        while j < bytes.len() {
            if bytes[j] == b'{' && !is_escaped_at_position(bytes, j) {
                depth += 1;
            } else if bytes[j] == b'}' && !is_escaped_at_position(bytes, j) {
                depth -= 1;
                if depth == 0 {
                    mc = Some(j);
                    break;
                }
            }
            j += 1;
        }
        if let Some(cp) = mc {
            let mut id: i32 = 0;
            let mut k = i + 1;
            while k < cp {
                if bytes[k] == b'{' && !is_escaped_at_position(bytes, k) {
                    id += 1;
                } else if bytes[k] == b'}' && !is_escaped_at_position(bytes, k) {
                    id -= 1;
                } else if id == 0
                    && (bytes[k] == b','
                        || (bytes[k] == b'.' && k + 1 < cp && bytes[k + 1] == b'.'))
                {
                    return SecurityBehavior::Ask("Command contains brace expansion".into());
                }
                k += 1;
            }
        }
        i += 1;
    }
    SecurityBehavior::Passthrough
}

fn validate_comment_quote_desync(ctx: &ValidationContext) -> SecurityBehavior {
    let cmd = &ctx.original_command;
    let mut in_sq = false;
    let mut in_dq = false;
    let mut escaped = false;
    let chars: Vec<char> = cmd.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if escaped {
            escaped = false;
            i += 1;
            continue;
        }
        if in_sq {
            if ch == '\'' {
                in_sq = false;
            }
            i += 1;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            i += 1;
            continue;
        }
        if in_dq {
            if ch == '"' {
                in_dq = false;
            }
            i += 1;
            continue;
        }
        if ch == '\'' {
            in_sq = true;
            i += 1;
            continue;
        }
        if ch == '"' {
            in_dq = true;
            i += 1;
            continue;
        }
        if ch == '#' {
            let mut j = i + 1;
            while j < chars.len() && chars[j] != '\n' {
                if chars[j] == '\'' || chars[j] == '"' {
                    return SecurityBehavior::Ask(
                        "Command contains quotes inside # comment (desync)".into(),
                    );
                }
                j += 1;
            }
            i = j;
            continue;
        }
        i += 1;
    }
    SecurityBehavior::Passthrough
}

fn validate_quoted_newline(ctx: &ValidationContext) -> SecurityBehavior {
    let cmd = &ctx.original_command;
    if !cmd.contains('\n') || !cmd.contains('#') {
        return SecurityBehavior::Passthrough;
    }
    let mut in_sq = false;
    let mut in_dq = false;
    let mut escaped = false;
    let bytes = cmd.as_bytes();
    for i in 0..bytes.len() {
        let ch = bytes[i];
        if escaped {
            escaped = false;
            continue;
        }
        if ch == b'\\' && !in_sq {
            escaped = true;
            continue;
        }
        if ch == b'\'' && !in_dq {
            in_sq = !in_sq;
            continue;
        }
        if ch == b'"' && !in_sq {
            in_dq = !in_dq;
            continue;
        }
        if ch == b'\n' && (in_sq || in_dq) {
            let rest = &cmd[i + 1..];
            if let Some(line) = rest.lines().next() {
                if line.trim().starts_with('#') {
                    return SecurityBehavior::Ask(
                        "Command contains quoted newline followed by #-prefixed line".into(),
                    );
                }
            }
        }
    }
    SecurityBehavior::Passthrough
}

const ZSH_DANGEROUS_COMMANDS: &[&str] = &[
    "zmodload", "emulate", "sysopen", "sysread", "syswrite", "sysseek", "zpty", "ztcp", "zsocket",
    "mapfile", "zf_rm", "zf_mv", "zf_ln", "zf_chmod", "zf_chown", "zf_mkdir", "zf_rmdir",
    "zf_chgrp",
];

fn validate_zsh_dangerous_commands(ctx: &ValidationContext) -> SecurityBehavior {
    let trimmed = ctx.original_command.trim();
    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    let env_re = regex_lite::Regex::new(r"^[A-Za-z_]\w*=").unwrap();
    let zsh_mods = ["command", "builtin", "noglob", "nocorrect"];
    let mut base_cmd = "";
    for token in &tokens {
        if env_re.is_match(token) || zsh_mods.contains(token) {
            continue;
        }
        base_cmd = token;
        break;
    }
    if ZSH_DANGEROUS_COMMANDS.contains(&base_cmd) {
        return SecurityBehavior::Ask(format!("Command uses Zsh-specific '{}'", base_cmd));
    }
    if base_cmd == "fc"
        && regex_lite::Regex::new(r"\s-\S*e")
            .unwrap()
            .is_match(trimmed)
    {
        return SecurityBehavior::Ask("Command uses 'fc -e' (eval-equivalent)".into());
    }
    SecurityBehavior::Passthrough
}

// ---------------------------------------------------------------------------
// Full security validator chain
// ---------------------------------------------------------------------------

/// Run all security validators on a single (non-compound) command.
pub fn validate_command_security(command: &str) -> SecurityBehavior {
    if has_control_chars(command) {
        return SecurityBehavior::Ask("Command contains non-printable control characters".into());
    }
    let ctx = ValidationContext::new(command);

    // Early validators (can short-circuit with Allow)
    let early_validators: Vec<fn(&ValidationContext) -> SecurityBehavior> = vec![
        validate_empty,
        validate_incomplete_commands,
        validate_safe_command_substitution,
        validate_git_commit,
    ];
    for v in &early_validators {
        let r = v(&ctx);
        match &r {
            SecurityBehavior::Allow => {
                // TS returns passthrough for early-allow to let the command
                // proceed without further checks
                return SecurityBehavior::Passthrough;
            },
            SecurityBehavior::Ask(_) => return r,
            _ => {},
        }
    }

    // Misparsing validators (high priority)
    let misparsing: Vec<fn(&ValidationContext) -> SecurityBehavior> = vec![
        validate_jq_command,
        validate_obfuscated_flags,
        validate_shell_metacharacters,
        validate_dangerous_variables,
        validate_comment_quote_desync,
        validate_quoted_newline,
        validate_carriage_return,
        validate_ifs_injection,
        validate_proc_environ_access,
        validate_dangerous_patterns,
        validate_backslash_escaped_whitespace,
        validate_backslash_escaped_operators,
        validate_unicode_whitespace,
        validate_mid_word_hash,
        validate_brace_expansion,
        validate_zsh_dangerous_commands,
        validate_malformed_token_injection,
    ];
    for v in &misparsing {
        let r = v(&ctx);
        if let SecurityBehavior::Ask(_) = &r {
            return r;
        }
    }

    // Non-misparsing (deferred)
    let non_misp: Vec<fn(&ValidationContext) -> SecurityBehavior> =
        vec![validate_newlines, validate_redirections];
    let mut deferred: Option<SecurityBehavior> = None;
    for v in &non_misp {
        let r = v(&ctx);
        if let SecurityBehavior::Ask(_) = &r {
            if deferred.is_none() {
                deferred = Some(r);
            }
        }
    }
    if let Some(d) = deferred {
        return d;
    }

    SecurityBehavior::Passthrough
}

// ---------------------------------------------------------------------------
// Eval-like builtins
// ---------------------------------------------------------------------------

const EVAL_LIKE_BUILTINS: &[&str] = &[
    "eval",
    "source",
    ".",
    "exec",
    "command",
    "builtin",
    "fc",
    "coproc",
    "noglob",
    "nocorrect",
    "trap",
    "enable",
    "mapfile",
    "readarray",
    "hash",
    "bind",
    "complete",
    "compgen",
    "alias",
    "let",
];

pub fn check_eval_like_builtins(command_name: &str) -> bool {
    EVAL_LIKE_BUILTINS.contains(&command_name)
}

// ---------------------------------------------------------------------------
// Read-only / destructive command classification
// ---------------------------------------------------------------------------

pub const EXPANDED_READ_ONLY_COMMANDS: &[&str] = &[
    "ls",
    "cat",
    "head",
    "tail",
    "less",
    "more",
    "wc",
    "file",
    "stat",
    "readlink",
    "realpath",
    "du",
    "df",
    "lsof",
    "lsblk",
    "lscpu",
    "find",
    "grep",
    "egrep",
    "fgrep",
    "rg",
    "ag",
    "ack",
    "which",
    "where",
    "type",
    "whence",
    "whereis",
    "locate",
    "echo",
    "printf",
    "date",
    "pwd",
    "whoami",
    "hostname",
    "uname",
    "env",
    "printenv",
    "id",
    "groups",
    "true",
    "false",
    "test",
    "[",
    "git status",
    "git log",
    "git diff",
    "git show",
    "git branch",
    "git remote",
    "git tag",
    "git stash list",
    "git rev-parse",
    "git ls-files",
    "git ls-remote",
    "git ls-tree",
    "git describe",
    "git blame",
    "git shortlog",
    "git name-rev",
    "git rev-list",
    "git for-each-ref",
    "git count-objects",
    "git verify-commit",
    "git verify-tag",
    "git config --get",
    "git config --list",
    "git cat-file",
    "cargo check",
    "cargo test",
    "cargo clippy",
    "cargo build",
    "cargo doc",
    "cargo bench",
    "cargo metadata",
    "cargo tree",
    "npm test",
    "npm run lint",
    "npm run build",
    "npm run check",
    "npm ls",
    "npm list",
    "npm info",
    "npm view",
    "npm outdated",
    "npx tsc",
    "node -e",
    "node -p",
    "python -c",
    "python3 -c",
    "python -m py_compile",
    "pip list",
    "pip show",
    "pip freeze",
    "pip3 list",
    "pip3 show",
    "pip3 freeze",
    "gem list",
    "go list",
    "go vet",
    "go version",
    "rustup show",
    "rustc --version",
    "rustfmt --check",
    "docker ps",
    "docker images",
    "docker inspect",
    "docker logs",
    "docker version",
    "docker info",
    "docker stats",
    "docker-compose ps",
    "docker-compose logs",
    "uptime",
    "free",
    "vmstat",
    "ps",
    "pgrep",
    "nproc",
    "arch",
    "getconf",
    "dig",
    "host",
    "nslookup",
    "sort",
    "uniq",
    "tr",
    "cut",
    "paste",
    "column",
    "fmt",
    "fold",
    "expand",
    "unexpand",
    "nl",
    "diff",
    "comm",
    "cmp",
    "md5sum",
    "sha256sum",
    "sha1sum",
    "base64",
    "xxd",
    "od",
    "hexdump",
    "jq",
    "gh pr list",
    "gh pr view",
    "gh pr status",
    "gh issue list",
    "gh issue view",
    "gh issue status",
    "gh repo view",
    "gh api",
    "gh run list",
    "gh run view",
    "man",
    "help",
    "info",
    "cal",
    "bc",
    "basename",
    "dirname",
    "seq",
    "sleep",
    "tput",
    "clear",
    "reset",
    "xdg-open",
    "open",
];

const DESTRUCTIVE_COMMANDS: &[&str] = &[
    "rm",
    "rmdir",
    "mv",
    "cp",
    "chmod",
    "chown",
    "chgrp",
    "truncate",
    "shred",
    "mkdir",
    "touch",
    "ln",
    "kill",
    "killall",
    "pkill",
    "reboot",
    "shutdown",
    "halt",
    "poweroff",
    "mount",
    "umount",
    "useradd",
    "userdel",
    "usermod",
    "groupadd",
    "groupdel",
    "passwd",
    "chpasswd",
    "apt",
    "apt-get",
    "dpkg",
    "yum",
    "dnf",
    "pacman",
    "brew",
    "pip install",
    "pip3 install",
    "pip uninstall",
    "pip3 uninstall",
    "npm install",
    "npm uninstall",
    "npm update",
    "npm publish",
    "cargo install",
    "gem install",
    "gem uninstall",
    "curl",
    "wget",
    "sudo",
    "su",
    "doas",
    "git push",
    "git commit",
    "git add",
    "git rm",
    "git mv",
    "git reset",
    "git checkout",
    "git rebase",
    "git merge",
    "git cherry-pick",
    "git revert",
    "git clean",
    "git stash drop",
    "git stash pop",
    "git stash clear",
    "git branch -d",
    "git branch -D",
    "git push --force",
    "git push -f",
    "docker run",
    "docker exec",
    "docker build",
    "docker pull",
    "docker push",
    "docker rm",
    "docker rmi",
    "docker stop",
    "docker kill",
    "docker restart",
    "docker compose up",
    "dd",
    "mkfs",
    "fdisk",
    "parted",
    "iptables",
    "ip6tables",
    "nft",
    "systemctl",
    "service",
    "crontab",
];

// ---------------------------------------------------------------------------
// Safe env vars and wrapper stripping
// ---------------------------------------------------------------------------

const SAFE_ENV_VARS: &[&str] = &[
    "GOEXPERIMENT",
    "GOOS",
    "GOARCH",
    "CGO_ENABLED",
    "GO111MODULE",
    "RUST_BACKTRACE",
    "RUST_LOG",
    "NODE_ENV",
    "PYTHONUNBUFFERED",
    "PYTHONDONTWRITEBYTECODE",
    "PYTEST_DISABLE_PLUGIN_AUTOLOAD",
    "PYTEST_DEBUG",
    "ANTHROPIC_API_KEY",
    "LANG",
    "LANGUAGE",
    "LC_ALL",
    "LC_CTYPE",
    "LC_TIME",
    "CHARSET",
    "TERM",
    "COLORTERM",
    "NO_COLOR",
    "FORCE_COLOR",
    "TZ",
    "LS_COLORS",
    "LSCOLORS",
    "GREP_COLOR",
    "GREP_COLORS",
    "GCC_COLORS",
    "TIME_STYLE",
    "BLOCK_SIZE",
    "BLOCKSIZE",
];

pub fn strip_safe_wrappers(command: &str) -> String {
    let env_var_re =
        regex_lite::Regex::new(r"^([A-Za-z_][A-Za-z0-9_]*)=([A-Za-z0-9_./:-]+)\s+").unwrap();
    let wrapper_patterns = [
        regex_lite::Regex::new(r"^timeout\s+(?:(?:--(?:foreground|preserve-status|verbose)|--(?:kill-after|signal)=[A-Za-z0-9_.+-]+|-v|-[ks]\s+[A-Za-z0-9_.+-]+|-[ks][A-Za-z0-9_.+-]+)\s+)*(?:--\s+)?\d+(?:\.\d+)?[smhd]?\s+").unwrap(),
        regex_lite::Regex::new(r"^time\s+(?:--\s+)?").unwrap(),
        regex_lite::Regex::new(r"^nice(?:\s+-n\s+-?\d+|\s+-\d+)?\s+(?:--\s+)?").unwrap(),
        regex_lite::Regex::new(r"^stdbuf(?:\s+-[ioe][LN0-9]+)+\s+(?:--\s+)?").unwrap(),
        regex_lite::Regex::new(r"^nohup\s+(?:--\s+)?").unwrap(),
    ];
    let mut stripped = command.to_string();
    let mut prev = String::new();
    while stripped != prev {
        prev.clone_from(&stripped);
        let lines: Vec<&str> = stripped
            .split('\n')
            .filter(|l| {
                let t = l.trim();
                !t.is_empty() && !t.starts_with('#')
            })
            .collect();
        if !lines.is_empty() {
            stripped = lines.join("\n");
        }
        if let Some(m) = env_var_re.find(&stripped) {
            let vn = stripped[..m.end()].split('=').next().unwrap_or("");
            if SAFE_ENV_VARS.contains(&vn) {
                stripped = stripped[m.end()..].to_string();
            }
        }
    }
    prev = String::new();
    while stripped != prev {
        prev.clone_from(&stripped);
        for pat in &wrapper_patterns {
            if let Some(m) = pat.find(&stripped) {
                stripped = stripped[m.end()..].to_string();
                break;
            }
        }
    }
    stripped
}

pub fn is_command_read_only(command: &str) -> bool {
    let stripped = strip_safe_wrappers(command);
    let cmd = stripped.trim();
    EXPANDED_READ_ONLY_COMMANDS
        .iter()
        .any(|safe| cmd == *safe || cmd.starts_with(&format!("{} ", safe)))
}

pub fn is_command_destructive(command: &str) -> bool {
    let stripped = strip_safe_wrappers(command);
    let cmd = stripped.trim();
    DESTRUCTIVE_COMMANDS
        .iter()
        .any(|d| cmd == *d || cmd.starts_with(&format!("{} ", d)))
}

/// Wildcard permission pattern matching.
pub fn match_wildcard_pattern(pattern: &str, command: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix(":*") {
        command == prefix || command.starts_with(&format!("{} ", prefix))
    } else {
        command == pattern
    }
}

/// Extract a stable command prefix (command + subcommand).
pub fn get_command_prefix(command: &str) -> Option<String> {
    let env_var_re = regex_lite::Regex::new(r"^[A-Za-z_]\w*=").unwrap();
    let tokens: Vec<&str> = command.split_whitespace().collect();
    let mut i = 0;
    while i < tokens.len() && env_var_re.is_match(tokens[i]) {
        let vn = tokens[i].split('=').next().unwrap_or("");
        if !SAFE_ENV_VARS.contains(&vn) {
            return None;
        }
        i += 1;
    }
    let remaining = &tokens[i..];
    if remaining.len() < 2 {
        return None;
    }
    if !regex_lite::Regex::new(r"^[a-z][a-z0-9]*(-[a-z0-9]+)*$")
        .unwrap()
        .is_match(remaining[1])
    {
        return None;
    }
    Some(format!("{} {}", remaining[0], remaining[1]))
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_control_chars() {
        assert!(has_control_chars("echo \x00hello"));
        assert!(!has_control_chars("echo hello"));
    }
    #[test]
    fn test_unicode_ws() {
        assert!(has_unicode_whitespace("echo\u{00A0}hello"));
        assert!(!has_unicode_whitespace("echo hello"));
    }
    #[test]
    fn test_zsh_tilde() {
        assert!(has_zsh_tilde_bracket("~[foo]"));
        assert!(!has_zsh_tilde_bracket("~/foo"));
    }
    #[test]
    fn test_zsh_equals() {
        assert!(has_zsh_equals_expansion("=curl evil"));
        assert!(!has_zsh_equals_expansion("VAR=val cmd"));
    }

    #[test]
    fn test_empty() {
        assert_eq!(
            validate_empty(&ValidationContext::new("")),
            SecurityBehavior::Allow
        );
    }
    #[test]
    fn test_incomplete() {
        assert!(matches!(
            validate_incomplete_commands(&ValidationContext::new("-rf")),
            SecurityBehavior::Ask(_)
        ));
    }
    #[test]
    fn test_cmd_sub() {
        assert!(matches!(
            validate_dangerous_patterns(&ValidationContext::new("echo $(id)")),
            SecurityBehavior::Ask(_)
        ));
    }
    #[test]
    fn test_backtick() {
        assert!(matches!(
            validate_dangerous_patterns(&ValidationContext::new("echo `id`")),
            SecurityBehavior::Ask(_)
        ));
    }
    #[test]
    fn test_proc_sub() {
        assert!(matches!(
            validate_dangerous_patterns(&ValidationContext::new("diff <(a) <(b)")),
            SecurityBehavior::Ask(_)
        ));
    }
    #[test]
    fn test_ifs() {
        assert!(matches!(
            validate_ifs_injection(&ValidationContext::new("cmd$IFSarg")),
            SecurityBehavior::Ask(_)
        ));
    }
    #[test]
    fn test_proc() {
        assert!(matches!(
            validate_proc_environ_access(&ValidationContext::new("cat /proc/self/environ")),
            SecurityBehavior::Ask(_)
        ));
    }
    #[test]
    fn test_cr() {
        assert!(matches!(
            validate_carriage_return(&ValidationContext::new("TZ=UTC\recho evil")),
            SecurityBehavior::Ask(_)
        ));
    }
    #[test]
    fn test_cr_dq_safe() {
        assert_eq!(
            validate_carriage_return(&ValidationContext::new("echo \"x\ry\"")),
            SecurityBehavior::Passthrough
        );
    }
    #[test]
    fn test_redir() {
        assert!(matches!(
            validate_redirections(&ValidationContext::new("echo > /tmp/f")),
            SecurityBehavior::Ask(_)
        ));
    }
    #[test]
    fn test_redir_devnull() {
        assert_eq!(
            validate_redirections(&ValidationContext::new("echo > /dev/null")),
            SecurityBehavior::Passthrough
        );
    }
    #[test]
    fn test_ansi_c() {
        assert!(matches!(
            validate_obfuscated_flags(&ValidationContext::new("find $'-exec' rm {} +")),
            SecurityBehavior::Ask(_)
        ));
    }
    #[test]
    fn test_bs_ws() {
        assert!(matches!(
            validate_backslash_escaped_whitespace(&ValidationContext::new("echo\\ test")),
            SecurityBehavior::Ask(_)
        ));
    }
    #[test]
    fn test_bs_op() {
        assert!(matches!(
            validate_backslash_escaped_operators(&ValidationContext::new(
                "cat f \\; echo /etc/passwd"
            )),
            SecurityBehavior::Ask(_)
        ));
    }
    #[test]
    fn test_brace_comma() {
        assert!(matches!(
            validate_brace_expansion(&ValidationContext::new("echo {a,b}")),
            SecurityBehavior::Ask(_)
        ));
    }
    #[test]
    fn test_brace_seq() {
        assert!(matches!(
            validate_brace_expansion(&ValidationContext::new("echo {1..10}")),
            SecurityBehavior::Ask(_)
        ));
    }
    #[test]
    fn test_zsh_zmodload() {
        assert!(matches!(
            validate_zsh_dangerous_commands(&ValidationContext::new("zmodload zsh/system")),
            SecurityBehavior::Ask(_)
        ));
    }
    #[test]
    fn test_fc_e() {
        assert!(matches!(
            validate_zsh_dangerous_commands(&ValidationContext::new("fc -e vim")),
            SecurityBehavior::Ask(_)
        ));
    }
    #[test]
    fn test_jq_system() {
        assert!(matches!(
            validate_jq_command(&ValidationContext::new("jq 'system(\"rm\")'")),
            SecurityBehavior::Ask(_)
        ));
    }
    #[test]
    fn test_jq_flag() {
        assert!(matches!(
            validate_jq_command(&ValidationContext::new("jq -f evil.jq in")),
            SecurityBehavior::Ask(_)
        ));
    }

    #[test]
    fn test_eval_like() {
        assert!(check_eval_like_builtins("eval"));
        assert!(!check_eval_like_builtins("echo"));
    }
    #[test]
    fn test_read_only() {
        assert!(is_command_read_only("ls -la"));
        assert!(is_command_read_only("git status"));
        assert!(!is_command_read_only("rm -rf /"));
    }
    #[test]
    fn test_destructive() {
        assert!(is_command_destructive("rm -rf /"));
        assert!(!is_command_destructive("ls"));
    }
    #[test]
    fn test_wc_exact() {
        assert!(match_wildcard_pattern("git status", "git status"));
        assert!(!match_wildcard_pattern("git status", "git status -v"));
    }
    #[test]
    fn test_wc_prefix() {
        assert!(match_wildcard_pattern("npm run:*", "npm run build"));
        assert!(!match_wildcard_pattern("npm run:*", "npm install"));
    }
    #[test]
    fn test_strip_nohup() {
        assert_eq!(strip_safe_wrappers("nohup git push"), "git push");
    }
    #[test]
    fn test_strip_env() {
        assert_eq!(
            strip_safe_wrappers("NODE_ENV=prod npm run build"),
            "npm run build"
        );
    }
    #[test]
    fn test_prefix() {
        assert_eq!(
            get_command_prefix("git commit -m 'x'"),
            Some("git commit".into())
        );
        assert_eq!(get_command_prefix("ls -la"), None);
    }

    #[test]
    fn test_sec_safe() {
        assert_eq!(
            validate_command_security("ls -la"),
            SecurityBehavior::Passthrough
        );
    }
    // Empty commands get Allow from validate_empty, which the chain maps to Passthrough
    #[test]
    fn test_sec_empty() {
        assert_eq!(validate_command_security(""), SecurityBehavior::Passthrough);
    }
    #[test]
    fn test_sec_control() {
        assert!(matches!(
            validate_command_security("echo \x01w"),
            SecurityBehavior::Ask(_)
        ));
    }
    #[test]
    fn test_sec_backtick() {
        assert!(matches!(
            validate_command_security("echo `id`"),
            SecurityBehavior::Ask(_)
        ));
    }
    #[test]
    fn test_sec_dollar() {
        assert!(matches!(
            validate_command_security("echo $(whoami)"),
            SecurityBehavior::Ask(_)
        ));
    }

    #[test]
    fn test_unescaped_bt() {
        assert!(has_unescaped_char("test `d`", '`'));
        assert!(!has_unescaped_char("test \\`s\\`", '`'));
    }
    #[test]
    fn test_escaped_pos() {
        assert!(is_escaped_at_position(b"x\\{y", 2));
        assert!(!is_escaped_at_position(b"x\\\\{y", 4));
    }

    // -- Git commit early-allow --
    #[test]
    fn test_git_commit_simple_allow() {
        // Simple git commit should be allowed early
        assert_eq!(
            validate_git_commit(&ValidationContext::new("git commit -m 'fix typo'")),
            SecurityBehavior::Allow
        );
    }
    #[test]
    fn test_git_commit_dq_allow() {
        assert_eq!(
            validate_git_commit(&ValidationContext::new("git commit -m \"fix bug\"")),
            SecurityBehavior::Allow
        );
    }
    #[test]
    fn test_git_commit_cmd_sub_ask() {
        assert!(matches!(
            validate_git_commit(&ValidationContext::new("git commit -m \"$(evil)\"")),
            SecurityBehavior::Ask(_)
        ));
    }
    #[test]
    fn test_git_commit_backslash_passthrough() {
        // Backslashes in git commit should passthrough for full validation
        assert_eq!(
            validate_git_commit(&ValidationContext::new("git commit -m 'test\\msg'")),
            SecurityBehavior::Passthrough
        );
    }
    #[test]
    fn test_git_commit_semicolon_passthrough() {
        // Remainder with metacharacters should passthrough
        assert_eq!(
            validate_git_commit(&ValidationContext::new("git commit -m 'x'; evil")),
            SecurityBehavior::Passthrough
        );
    }
    #[test]
    fn test_git_commit_not_git() {
        assert_eq!(
            validate_git_commit(&ValidationContext::new("echo hello")),
            SecurityBehavior::Passthrough
        );
    }

    // -- Malformed token injection --
    #[test]
    fn test_malformed_balanced() {
        assert_eq!(
            validate_malformed_token_injection(&ValidationContext::new(
                "echo 'hello'; echo 'world'"
            )),
            SecurityBehavior::Passthrough
        );
    }
    #[test]
    fn test_malformed_unbalanced() {
        // Unbalanced parens with separator
        assert!(matches!(
            validate_malformed_token_injection(&ValidationContext::new("echo (hi; evil")),
            SecurityBehavior::Ask(_)
        ));
    }
    #[test]
    fn test_malformed_unbalanced_quotes() {
        // Odd number of quotes with separator
        assert!(matches!(
            validate_malformed_token_injection(&ValidationContext::new("echo 'hi; evil")),
            SecurityBehavior::Ask(_)
        ));
    }
    #[test]
    fn test_malformed_no_separator() {
        // Without separator, even unbalanced is OK (no injection vector)
        assert_eq!(
            validate_malformed_token_injection(&ValidationContext::new("echo (hi")),
            SecurityBehavior::Passthrough
        );
    }

    // -- Full chain includes new validators --
    #[test]
    fn test_sec_git_commit_allowed() {
        // git commit -m 'msg' should pass all security checks
        assert_eq!(
            validate_command_security("git commit -m 'fix typo'"),
            SecurityBehavior::Passthrough
        );
    }
    #[test]
    fn test_sec_malformed_caught() {
        assert!(matches!(
            validate_command_security("echo (hi; evil"),
            SecurityBehavior::Ask(_)
        ));
    }
}
