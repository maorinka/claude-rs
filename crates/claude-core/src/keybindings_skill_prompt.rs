//! `/keybindings-help` bundled-skill static sections.
//!
//! Port of TS `src/skills/bundled/keybindings.ts`. Guides the
//! model through editing `~/.claude/keybindings.json`.
//!
//! # Scope
//!
//! TS assembles the final prompt from 8 static SECTION_* blocks
//! plus 3 dynamic reference tables (reserved shortcuts, contexts,
//! actions). The static sections are reproduced verbatim here
//! as public constants; the dynamic tables depend on
//! keybinding-infrastructure constants
//! (`KEYBINDING_ACTIONS`, `KEYBINDING_CONTEXTS`, `DEFAULT_BINDINGS`,
//! `NON_REBINDABLE`, `TERMINAL_RESERVED`, `MACOS_RESERVED`) that
//! aren't ported to Rust yet, so those stay on the caller until
//! the keybinding system lands.
//!
//! When the rest of the infrastructure arrives, callers can
//! assemble the full skill prompt via [`assemble_keybindings_prompt`]
//! which accepts the three dynamic tables as strings.

/// Intro + read-before-write rule. Port of TS `SECTION_INTRO`
/// (keybindings.ts:149-160).
pub const KEYBINDINGS_SECTION_INTRO: &str = "# Keybindings Skill

Create or modify `~/.claude/keybindings.json` to customize keyboard shortcuts.

## CRITICAL: Read Before Write

**Always read `~/.claude/keybindings.json` first** (it may not exist yet). Merge changes with existing bindings — never replace the entire file.

- Use **Edit** tool for modifications to existing files
- Use **Write** tool only if the file does not exist yet";

/// File-format example + `$schema`/`$docs` guidance. Port of TS
/// `SECTION_FILE_FORMAT` (keybindings.ts:162-170).
pub const KEYBINDINGS_SECTION_FILE_FORMAT: &str = "## File Format

```json
{
  \"$schema\": \"https://www.schemastore.org/claude-code-keybindings.json\",
  \"$docs\": \"https://code.claude.com/docs/en/keybindings\",
  \"bindings\": [
    {
      \"context\": \"Chat\",
      \"bindings\": {
        \"ctrl+e\": \"chat:externalEditor\"
      }
    }
  ]
}
```

Always include the `$schema` and `$docs` fields.";

/// Keystroke syntax cheat sheet (modifiers, special keys, chords).
/// Port of TS `SECTION_KEYSTROKE_SYNTAX` (keybindings.ts:172-186).
pub const KEYBINDINGS_SECTION_KEYSTROKE_SYNTAX: &str = "## Keystroke Syntax

**Modifiers** (combine with `+`):
- `ctrl` (alias: `control`)
- `alt` (aliases: `opt`, `option`) — note: `alt` and `meta` are identical in terminals
- `shift`
- `meta` (aliases: `cmd`, `command`)

**Special keys**: `escape`/`esc`, `enter`/`return`, `tab`, `space`, `backspace`, `delete`, `up`, `down`, `left`, `right`

**Chords**: Space-separated keystrokes, e.g. `ctrl+k ctrl+s` (1-second timeout between keystrokes)

**Examples**: `ctrl+shift+p`, `alt+enter`, `ctrl+k ctrl+n`";

/// `null`-to-unbind example. Port of TS `SECTION_UNBINDING`
/// (keybindings.ts:188-196).
pub const KEYBINDINGS_SECTION_UNBINDING: &str = "## Unbinding Default Shortcuts

Set a key to `null` to remove its default binding:

```json
{
  \"context\": \"Chat\",
  \"bindings\": {
    \"ctrl+s\": null
  }
}
```";

/// How additive user bindings layer over defaults. Port of TS
/// `SECTION_INTERACTION` (keybindings.ts:198-204).
pub const KEYBINDINGS_SECTION_INTERACTION: &str = "## How User Bindings Interact with Defaults

- User bindings are **additive** — they are appended after the default bindings
- To **move** a binding to a different key: unbind the old key (`null`) AND add the new binding
- A context only needs to appear in the user's file if they want to change something in that context";

/// Rebind + chord patterns. Port of TS `SECTION_COMMON_PATTERNS`
/// (keybindings.ts:206-219).
pub const KEYBINDINGS_SECTION_COMMON_PATTERNS: &str = "## Common Patterns

### Rebind a key
To change the external editor shortcut from `ctrl+g` to `ctrl+e`:
```json
{
  \"context\": \"Chat\",
  \"bindings\": {
    \"ctrl+g\": null,
    \"ctrl+e\": \"chat:externalEditor\"
  }
}
```

### Add a chord binding
```json
{
  \"context\": \"Global\",
  \"bindings\": {
    \"ctrl+k ctrl+t\": \"app:toggleTodos\"
  }
}
```";

/// Behavioral rules (validation, tmux/screen warnings, additive
/// semantics). Port of TS `SECTION_BEHAVIORAL_RULES`
/// (keybindings.ts:221-229).
pub const KEYBINDINGS_SECTION_BEHAVIORAL_RULES: &str = "## Behavioral Rules

1. Only include contexts the user wants to change (minimal overrides)
2. Validate that actions and contexts are from the known lists below
3. Warn the user proactively if they choose a key that conflicts with reserved shortcuts or common tools like tmux (`ctrl+b`) and screen (`ctrl+a`)
4. When adding a new binding for an existing action, the new binding is additive (existing default still works unless explicitly unbound)
5. To fully replace a default binding, unbind the old key AND add the new one";

/// `/doctor` validation reference — common issues table + sample
/// doctor output. Port of TS `SECTION_DOCTOR`
/// (keybindings.ts:231-290).
pub const KEYBINDINGS_SECTION_DOCTOR: &str = "## Validation with /doctor

The `/doctor` command includes a \"Keybinding Configuration Issues\" section that validates `~/.claude/keybindings.json`.

### Common Issues and Fixes

| Issue | Cause | Fix |
| --- | --- | --- |
| `keybindings.json must have a \"bindings\" array` | Missing wrapper object | Wrap bindings in `{ \"bindings\": [...] }` |
| `\"bindings\" must be an array` | `bindings` is not an array | Set `\"bindings\"` to an array: `[{ context: ..., bindings: ... }]` |
| `Unknown context \"X\"` | Typo or invalid context name | Use exact context names from the Available Contexts table |
| `Duplicate key \"X\" in Y bindings` | Same key defined twice in one context | Remove the duplicate; JSON uses only the last value |
| `\"X\" may not work: ...` | Key conflicts with terminal/OS reserved shortcut | Choose a different key (see Reserved Shortcuts section) |
| `Could not parse keystroke \"X\"` | Invalid key syntax | Check syntax: use `+` between modifiers, valid key names |
| `Invalid action for \"X\"` | Action value is not a string or null | Actions must be strings like `\"app:help\"` or `null` to unbind |

### Example /doctor Output

```
Keybinding Configuration Issues
Location: ~/.claude/keybindings.json
  └ [Error] Unknown context \"chat\"
    → Valid contexts: Global, Chat, Autocomplete, ...
  └ [Warning] \"ctrl+c\" may not work: Terminal interrupt (SIGINT)
```

**Errors** prevent bindings from working and must be fixed. **Warnings** indicate potential conflicts but the binding may still work.";

/// Build a markdown table from headers + rows. Port of TS
/// `markdownTable` (keybindings.ts:332-339). Exposed so callers
/// rendering the three dynamic tables (reserved shortcuts,
/// contexts, actions) reuse the same formatter the TS skill
/// uses.
pub fn markdown_table(headers: &[&str], rows: &[&[&str]]) -> String {
    let mut out = String::new();
    out.push_str("| ");
    out.push_str(&headers.join(" | "));
    out.push_str(" |\n| ");
    out.push_str(&vec!["---"; headers.len()].join(" | "));
    out.push_str(" |");
    for row in rows {
        out.push_str("\n| ");
        out.push_str(&row.join(" | "));
        out.push_str(" |");
    }
    out
}

/// Assemble the full keybindings-skill prompt. Port of TS
/// `getPromptForCommand(args)` at keybindings.ts:300-325.
///
/// Caller supplies the three dynamic table strings (rendered
/// from keybinding-infrastructure arrays that aren't ported to
/// Rust yet) + optional user-request text. Pass empty strings to
/// elide any block the caller can't produce — the prompt stays
/// structurally intact without empty headers.
pub struct KeybindingsPromptInputs<'a> {
    /// Pre-rendered reserved-shortcuts markdown (headings + bullets).
    pub reserved_shortcuts: &'a str,
    /// Pre-rendered contexts table.
    pub contexts_table: &'a str,
    /// Pre-rendered actions table.
    pub actions_table: &'a str,
    /// User's free-form request (empty to suppress the section).
    pub args: &'a str,
}

/// Join every section into the final prompt body. See
/// [`KeybindingsPromptInputs`].
pub fn assemble_keybindings_prompt(inputs: &KeybindingsPromptInputs<'_>) -> String {
    let mut sections: Vec<String> = vec![
        KEYBINDINGS_SECTION_INTRO.to_string(),
        KEYBINDINGS_SECTION_FILE_FORMAT.to_string(),
        KEYBINDINGS_SECTION_KEYSTROKE_SYNTAX.to_string(),
        KEYBINDINGS_SECTION_UNBINDING.to_string(),
        KEYBINDINGS_SECTION_INTERACTION.to_string(),
        KEYBINDINGS_SECTION_COMMON_PATTERNS.to_string(),
        KEYBINDINGS_SECTION_BEHAVIORAL_RULES.to_string(),
        KEYBINDINGS_SECTION_DOCTOR.to_string(),
    ];
    if !inputs.reserved_shortcuts.is_empty() {
        sections.push(format!(
            "## Reserved Shortcuts\n\n{}",
            inputs.reserved_shortcuts
        ));
    }
    if !inputs.contexts_table.is_empty() {
        sections.push(format!(
            "## Available Contexts\n\n{}",
            inputs.contexts_table
        ));
    }
    if !inputs.actions_table.is_empty() {
        sections.push(format!("## Available Actions\n\n{}", inputs.actions_table));
    }
    if !inputs.args.is_empty() {
        sections.push(format!("## User Request\n\n{}", inputs.args));
    }
    sections.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs_with<'a>(args: &'a str) -> KeybindingsPromptInputs<'a> {
        KeybindingsPromptInputs {
            reserved_shortcuts: "",
            contexts_table: "",
            actions_table: "",
            args,
        }
    }

    #[test]
    fn every_static_section_has_heading() {
        for (section, heading) in [
            (KEYBINDINGS_SECTION_INTRO, "# Keybindings Skill"),
            (KEYBINDINGS_SECTION_FILE_FORMAT, "## File Format"),
            (KEYBINDINGS_SECTION_KEYSTROKE_SYNTAX, "## Keystroke Syntax"),
            (
                KEYBINDINGS_SECTION_UNBINDING,
                "## Unbinding Default Shortcuts",
            ),
            (
                KEYBINDINGS_SECTION_INTERACTION,
                "## How User Bindings Interact with Defaults",
            ),
            (KEYBINDINGS_SECTION_COMMON_PATTERNS, "## Common Patterns"),
            (KEYBINDINGS_SECTION_BEHAVIORAL_RULES, "## Behavioral Rules"),
            (KEYBINDINGS_SECTION_DOCTOR, "## Validation with /doctor"),
        ] {
            assert!(section.contains(heading), "section missing `{heading}`");
        }
    }

    #[test]
    fn markdown_table_formats_header_and_rows() {
        let t = markdown_table(
            &["Foo", "Bar"],
            &[&["a", "b"] as &[&str], &["c", "d"] as &[&str]],
        );
        assert!(t.starts_with("| Foo | Bar |\n| --- | --- |"));
        assert!(t.contains("| a | b |"));
        assert!(t.contains("| c | d |"));
    }

    #[test]
    fn markdown_table_single_column() {
        let t = markdown_table(&["Only"], &[&["one"] as &[&str]]);
        assert!(t.contains("| Only |\n| --- |\n| one |"));
    }

    #[test]
    fn assemble_emits_static_sections_without_dynamic_tables() {
        let p = assemble_keybindings_prompt(&inputs_with(""));
        assert!(p.starts_with("# Keybindings Skill"));
        assert!(p.contains("## File Format"));
        assert!(p.contains("## Validation with /doctor"));
        // No dynamic tables → those headers are omitted.
        assert!(!p.contains("## Reserved Shortcuts"));
        assert!(!p.contains("## Available Contexts"));
        assert!(!p.contains("## Available Actions"));
        assert!(!p.contains("## User Request"));
    }

    #[test]
    fn assemble_appends_dynamic_tables_when_present() {
        let inputs = KeybindingsPromptInputs {
            reserved_shortcuts: "- `ctrl+c` — SIGINT",
            contexts_table: "| Context | Description |\n| --- | --- |",
            actions_table: "| Action | Keys | Context |\n| --- | --- | --- |",
            args: "",
        };
        let p = assemble_keybindings_prompt(&inputs);
        assert!(p.contains("## Reserved Shortcuts\n\n- `ctrl+c` — SIGINT"));
        assert!(p.contains("## Available Contexts\n\n| Context"));
        assert!(p.contains("## Available Actions\n\n| Action"));
    }

    #[test]
    fn assemble_appends_user_request_when_args_present() {
        let p = assemble_keybindings_prompt(&inputs_with("rebind ctrl+s"));
        assert!(p.ends_with("## User Request\n\nrebind ctrl+s"));
    }

    #[test]
    fn doctor_section_lists_every_common_issue_category() {
        let d = KEYBINDINGS_SECTION_DOCTOR;
        for anchor in &[
            "must have a \"bindings\" array",
            "Unknown context",
            "Duplicate key",
            "may not work",
            "Could not parse keystroke",
            "Invalid action",
        ] {
            assert!(d.contains(anchor), "doctor section missing `{anchor}`");
        }
    }

    #[test]
    fn common_patterns_demonstrates_rebind_and_chord() {
        let s = KEYBINDINGS_SECTION_COMMON_PATTERNS;
        assert!(s.contains("### Rebind a key"));
        assert!(s.contains("### Add a chord binding"));
        assert!(s.contains("chat:externalEditor"));
        assert!(s.contains("app:toggleTodos"));
    }
}
