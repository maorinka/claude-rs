#!/usr/bin/env python3
"""Manual overrides for audit entries that are ported in Rust
but the auto-detector missed (usually because the Rust prompt
lives in a separate `.md` or the TS template has heavy
interpolation that breaks substring matches).

Each override says: for section `title` in `file`, if current
Status is `NOT IN RUST`, flip to `FOUND in Rust` with the
listed Rust path(s).
"""

import re
from pathlib import Path

ROOT = Path("/Users/maorhadad/projects/claude/claude-rs")

# (title_prefix, rust_paths) — the title matches the `### {prefix}` line.
OVERRIDES = [
    ("Config Tool Prompt", ["crates/claude-tools/src/prompts/config_tool.md"]),
    ("AskUserQuestion Tool Preview Feature Prompt", ["crates/claude-tools/src/ask_user.rs"]),
    ("Bash Tool - Git Commit and PR Instructions (External Users)", ["crates/claude-tools/src/bash.rs"]),
    ("Bash Tool - Git Instructions for Ant Users (Short Version)", ["crates/claude-tools/src/bash.rs"]),
    ("EnterPlanMode Tool Prompt (Ant Users)", ["crates/claude-tools/src/plan_mode.rs"]),
    ("PowerShell Tool - Edition Section (PS 5.1 vs 7+)", ["crates/claude-tools/src/prompts/powershell.md"]),
    ("CronCreate Tool Description", ["crates/claude-tools/src/cron_tool.rs"]),
    ("MCP Auth Tool - Dynamic Description", ["crates/claude-tools/src/mcp_auth_tool.rs"]),
    ("Verification Agent Critical System Reminder", ["crates/claude-tools/src/agents/"]),
    ("Claude Code Guide Agent - Dynamic context appended to system prompt", ["crates/claude-tools/src/agents/"]),
]

DOCS = [
    "prompts_part1_tools.md",
    "prompts_part2_commands_hooks.md",
    "prompts_part3_services_skills.md",
    "prompts_part4_utils_query_constants.md",
    "prompts_part5_components_bridge_rest.md",
    "ALL_PROMPTS.md",
    "ALL_PROMPTS_PDF.md",
]

STATUS_PAT = re.compile(
    r"^\*\*Status:\s*(?:❌\s*)?NOT IN RUST\*\*[^\n]*$", re.MULTILINE
)


def flip(text: str, title_prefix: str, rust_paths: list[str]) -> tuple[str, int]:
    # Find section starting with `### {title_prefix}` (allow
    # tail text after prefix, e.g. line-wrap).
    sections = re.split(r"(?=^### )", text, flags=re.MULTILINE)
    out = []
    flipped = 0
    for sec in sections:
        header_line = sec.split("\n", 1)[0] if sec.startswith("### ") else ""
        if header_line.startswith(f"### {title_prefix}"):
            new_status = (
                "**Status: ✅ FOUND in Rust** — "
                + ", ".join(f"`{p}`" for p in rust_paths)
            )
            new_sec, n = STATUS_PAT.subn(new_status, sec, count=1)
            if n:
                flipped += 1
            out.append(new_sec)
        else:
            out.append(sec)
    return "".join(out), flipped


def main():
    for doc in DOCS:
        p = ROOT / doc
        if not p.exists():
            continue
        text = p.read_text(encoding="utf-8")
        total = 0
        for title, paths in OVERRIDES:
            text, n = flip(text, title, paths)
            total += n
        if total:
            p.write_text(text, encoding="utf-8")
            print(f"{doc}: +{total}")


if __name__ == "__main__":
    main()
