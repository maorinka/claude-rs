//! Sandbox section for the BashTool prompt.
//!
//! Port of the TS `getSimpleSandboxSection()` from
//! `src/tools/BashTool/prompt.ts`. The TS side dynamically
//! assembles this block from the current SandboxManager config
//! (filesystem read/write allow/deny, network allowed/denied hosts,
//! ignoreViolations, allowUnixSockets, per-UID claudeTempDir →
//! `$TMPDIR` normalisation) and splices it into the Bash tool's
//! description at runtime.
//!
//! The Rust port here covers the STATIC half — the explanatory
//! header + sandbox-override guidance + TMPDIR note — with a
//! `{SANDBOX_RESTRICTIONS}` placeholder where the dynamic
//! Filesystem / Network / Ignored-violations lines should be
//! inserted. Callers that want the dynamic block:
//! 1. `format_sandbox_section(restrictions)` — formats the full
//!    section with the supplied restrictions lines.
//! 2. Splice the returned string into the BashTool description
//!    at system-prompt-build time.
//!
//! Embedding the restrictions at build time keeps the Rust
//! BashTool description static-`&str` compatible (important for
//! prompt-cache prefix stability) while still letting advanced
//! callers customise when they need to.

/// Static TS `getSimpleSandboxSection()` template with
/// `{SANDBOX_RESTRICTIONS}` placeholder.
pub const BASH_SANDBOX_SECTION_TEMPLATE: &str = include_str!("prompts/bash_sandbox_section.md");

/// Format the sandbox section by substituting `restrictions` for
/// the `{SANDBOX_RESTRICTIONS}` placeholder. `restrictions` should
/// be the newline-joined set of restriction lines (e.g.
/// `"Filesystem: {...}\nNetwork: {...}"`). When empty, the
/// placeholder is replaced with an empty string.
pub fn format_sandbox_section(restrictions: &str) -> String {
    BASH_SANDBOX_SECTION_TEMPLATE.replace("{SANDBOX_RESTRICTIONS}", restrictions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_contains_placeholder() {
        assert!(BASH_SANDBOX_SECTION_TEMPLATE.contains("{SANDBOX_RESTRICTIONS}"));
    }

    #[test]
    fn template_has_stable_header() {
        // Load-bearing against prompt-cache prefix: the "Command
        // sandbox" header line is what the model's system prompt
        // keys on when cross-referencing sandbox guidance.
        assert!(BASH_SANDBOX_SECTION_TEMPLATE.contains("## Command sandbox"));
    }

    #[test]
    fn format_substitutes_restrictions() {
        let out = format_sandbox_section("Filesystem: {\"read\":{}, \"write\":{}}\nNetwork: {}");
        assert!(!out.contains("{SANDBOX_RESTRICTIONS}"));
        assert!(out.contains("Filesystem:"));
        assert!(out.contains("Network:"));
    }

    #[test]
    fn format_with_empty_restrictions_keeps_section() {
        let out = format_sandbox_section("");
        assert!(!out.contains("{SANDBOX_RESTRICTIONS}"));
        assert!(out.contains("## Command sandbox"));
    }

    #[test]
    fn format_preserves_key_guidance_items() {
        let out = format_sandbox_section("");
        assert!(out.contains("dangerouslyDisableSandbox"));
        assert!(out.contains("$TMPDIR"));
        assert!(out.contains("/sandbox"));
        assert!(out.contains("Operation not permitted"));
    }
}
