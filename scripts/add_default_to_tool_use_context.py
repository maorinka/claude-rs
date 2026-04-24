#!/usr/bin/env python3
"""Add `..Default::default()` to every `ToolUseContext { ... }` struct
literal that does not already have it or an `options:` / `host:` field.

Uses a brace-depth walker so nested `{}` inside field initializers don't
confuse the match.

Must distinguish STRUCT LITERALS from look-alikes that also contain the
token `ToolUseContext {`:

* `pub struct ToolUseContext { ... }`        — struct declaration
* `impl ToolUseContext { ... }`              — impl block
* `fn foo() -> ToolUseContext { ... }`       — function body whose return
                                               type is ToolUseContext
* `trait T: ToolUseContext { ... }`          — trait bound (rare)

In each non-literal case, the `{` opens a declaration or function body —
not an inline struct construction. Inserting `..Default::default()` into
those would wreck the file. The detector below inspects the non-whitespace
token immediately preceding `ToolUseContext` and skips the four look-alikes
above.
"""

import os
import re

ROOT = "/Users/maorhadad/projects/claude/claude-rs/crates"


def _prev_non_ws(text: str, idx: int) -> int:
    """Return the index of the nearest non-whitespace char strictly before idx,
    or -1 if none. Whitespace includes space, tab, and newline."""
    k = idx - 1
    while k >= 0 and text[k] in " \t\n\r":
        k -= 1
    return k


def _ends_with_keyword(text: str, end_idx: int, keyword: str) -> bool:
    """True if text[...end_idx+1] ends with `keyword` at an identifier boundary."""
    start = end_idx - len(keyword) + 1
    if start < 0:
        return False
    if text[start : end_idx + 1] != keyword:
        return False
    if start == 0:
        return True
    # Identifier boundary: the char before must not be alnum/underscore.
    prev = text[start - 1]
    return not (prev.isalnum() or prev == "_")


def is_non_literal_match(text: str, idx: int) -> bool:
    """True if `ToolUseContext {` at idx is NOT a struct literal.

    Recognizes struct declarations, impl blocks, trait bounds, and
    function return types.
    """
    k = _prev_non_ws(text, idx)
    if k < 0:
        return False
    # Function return type: `-> ToolUseContext {`
    if text[k] == ">" and k > 0 and text[k - 1] == "-":
        return True
    # Declaration/impl/trait keyword: `struct`, `impl`, `trait`
    for kw in ("struct", "impl", "trait"):
        if _ends_with_keyword(text, k, kw):
            return True
    return False


def add_default_to_literal(text: str) -> tuple[str, int]:
    out = []
    i = 0
    changes = 0
    while i < len(text):
        idx = text.find("ToolUseContext {", i)
        if idx < 0:
            out.append(text[i:])
            break

        # Non-literal look-alike: emit up to and past the token, then
        # keep scanning for the next occurrence.
        if is_non_literal_match(text, idx):
            token_end = idx + len("ToolUseContext ")
            out.append(text[i:token_end])
            i = token_end
            continue

        out.append(text[i:idx])
        brace_start = idx + len("ToolUseContext ")
        assert text[brace_start] == "{"
        depth = 1
        j = brace_start + 1
        while j < len(text) and depth > 0:
            c = text[j]
            if c == "{":
                depth += 1
            elif c == "}":
                depth -= 1
                if depth == 0:
                    break
            j += 1
        if depth != 0:
            out.append(text[idx:])
            break
        literal = text[idx : j + 1]
        body = literal[len("ToolUseContext {") : -1]
        if (
            "..Default::default()" in body
            or re.search(r"\bhost\s*:", body)
            or re.search(r"\boptions\s*:", body)
        ):
            out.append(literal)
            i = j + 1
            continue
        close_line_start = text.rfind("\n", 0, j) + 1
        close_indent = text[close_line_start:j]
        last_line_match = re.search(r"\n([ \t]+)[^\s]", body)
        field_indent = last_line_match.group(1) if last_line_match else close_indent + "    "
        body_rstripped = body.rstrip()
        if body_rstripped and not body_rstripped.endswith(","):
            body_rstripped += ","
        new_literal = (
            "ToolUseContext {"
            + body_rstripped
            + "\n"
            + field_indent
            + "..Default::default()\n"
            + close_indent
            + "}"
        )
        out.append(new_literal)
        changes += 1
        i = j + 1
    return "".join(out), changes


def main():
    total_changes = 0
    files_changed = 0
    for dirpath, dirs, files in os.walk(ROOT):
        dirs[:] = [d for d in dirs if d != "target"]
        for f in files:
            if not f.endswith(".rs"):
                continue
            path = os.path.join(dirpath, f)
            with open(path, "r", encoding="utf-8") as fp:
                text = fp.read()
            if "ToolUseContext {" not in text:
                continue
            new_text, changes = add_default_to_literal(text)
            if changes > 0 and new_text != text:
                with open(path, "w", encoding="utf-8") as fp:
                    fp.write(new_text)
                files_changed += 1
                total_changes += changes
                print(f"{path}: +{changes}")
    print(f"Total: {total_changes} sites updated across {files_changed} files")


if __name__ == "__main__":
    main()
