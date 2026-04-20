#!/usr/bin/env python3
"""Migrate every `ToolUseContext { working_directory: W,
read_file_state: R, permission_mode: P, ..Default::default() }`
struct literal to `ToolUseContext::for_test(W, R, P)`.

This is the second half of aligning `ToolUseContext` with the TS
original: TS has no `Default` escape hatch — construction sites
are explicit (TS `REPL.tsx:2392` for interactive,
`queryContext.ts:142` for headless). After this migration:

- Tests + headless sites use `ToolUseContext::for_test(...)`.
- Interactive sites will use `ToolUseContext::new(...)` once a
  real `InteractiveToolHost` + `ToolUseContextOptions` are wired
  through.

The matcher is strict: it only rewrites literals whose body is
exactly three-fields-then-spread in the canonical order. Anything
else is logged for manual handling.
"""

import os
import re

ROOT = "/Users/maorhadad/projects/claude/claude-rs/crates"


def _prev_non_ws(text: str, idx: int) -> int:
    k = idx - 1
    while k >= 0 and text[k] in " \t\n\r":
        k -= 1
    return k


def _ends_with_keyword(text: str, end_idx: int, keyword: str) -> bool:
    start = end_idx - len(keyword) + 1
    if start < 0:
        return False
    if text[start : end_idx + 1] != keyword:
        return False
    if start == 0:
        return True
    prev = text[start - 1]
    return not (prev.isalnum() or prev == "_")


def is_non_literal_match(text: str, idx: int) -> bool:
    k = _prev_non_ws(text, idx)
    if k < 0:
        return False
    if text[k] == ">" and k > 0 and text[k - 1] == "-":
        return True
    for kw in ("struct", "impl", "trait"):
        if _ends_with_keyword(text, k, kw):
            return True
    return False


def _split_top_level_commas(body: str) -> list[str]:
    """Split body on top-level commas, respecting `()[]{}` nesting."""
    parts = []
    buf = []
    depth_paren = 0
    depth_bracket = 0
    depth_brace = 0
    for c in body:
        if c == "(":
            depth_paren += 1
        elif c == ")":
            depth_paren -= 1
        elif c == "[":
            depth_bracket += 1
        elif c == "]":
            depth_bracket -= 1
        elif c == "{":
            depth_brace += 1
        elif c == "}":
            depth_brace -= 1
        elif c == "," and depth_paren == 0 and depth_bracket == 0 and depth_brace == 0:
            parts.append("".join(buf))
            buf = []
            continue
        buf.append(c)
    tail = "".join(buf).strip()
    if tail:
        parts.append(tail)
    return [p.strip() for p in parts]


def _extract_field(field_text: str, name: str) -> str | None:
    """If field_text is `name: <expr>`, return <expr>. Else None."""
    m = re.match(rf"^\s*{re.escape(name)}\s*:\s*", field_text)
    if not m:
        return None
    return field_text[m.end() :].rstrip()


def _try_rewrite(body: str) -> str | None:
    """Return the `for_test(W, R, P)` arg list if body matches the
    canonical three-fields-then-spread pattern, else None."""
    parts = _split_top_level_commas(body)
    if len(parts) != 4:
        return None
    if parts[3].replace(" ", "") != "..Default::default()":
        return None
    w = _extract_field(parts[0], "working_directory")
    r = _extract_field(parts[1], "read_file_state")
    p = _extract_field(parts[2], "permission_mode")
    if w is None or r is None or p is None:
        return None
    return f"{w}, {r}, {p}"


def migrate_literal(text: str) -> tuple[str, int, list[str]]:
    out = []
    i = 0
    changes = 0
    skipped: list[str] = []
    while i < len(text):
        idx = text.find("ToolUseContext {", i)
        if idx < 0:
            out.append(text[i:])
            break
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
        args = _try_rewrite(body)
        if args is None:
            skipped.append(literal[:200].replace("\n", " "))
            out.append(literal)
            i = j + 1
            continue
        out.append(f"ToolUseContext::for_test({args})")
        changes += 1
        i = j + 1
    return "".join(out), changes, skipped


def main():
    total_changes = 0
    files_changed = 0
    all_skipped: list[tuple[str, str]] = []
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
            new_text, changes, skipped = migrate_literal(text)
            for s in skipped:
                all_skipped.append((path, s))
            if changes > 0 and new_text != text:
                with open(path, "w", encoding="utf-8") as fp:
                    fp.write(new_text)
                files_changed += 1
                total_changes += changes
                print(f"{path}: +{changes}")
    print(f"Total: {total_changes} sites migrated across {files_changed} files")
    if all_skipped:
        print(f"\n{len(all_skipped)} sites skipped (manual handling needed):")
        for path, snippet in all_skipped:
            print(f"  {path}: {snippet}")


if __name__ == "__main__":
    main()
