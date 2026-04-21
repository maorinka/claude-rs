#!/usr/bin/env python3
"""Refresh the prompt-porting audit docs.

For every section currently marked `❌ NOT IN RUST`, pick a
characteristic string from its TS snippet (the reference code
block) and grep the Rust tree for it. If we find the string
verbatim in any .rs file, the section is actually ported — the
audit got stale.

Output:
- Per-file summary: how many stale entries, how many genuine.
- Rewritten doc with stale ones flipped to `✅ FOUND in Rust` +
  the file path(s) where the snippet lives.

Usage:
    python3 scripts/refresh_prompt_audit.py [--apply]

Without --apply, prints the plan; with --apply, writes changes
in-place. The rollup files (ALL_PROMPTS.md / ALL_PROMPTS_PDF.md)
are also refreshed.
"""

import argparse
import re
from pathlib import Path

ROOT = Path("/Users/maorhadad/projects/claude/claude-rs")
CRATES = ROOT / "crates"
DOCS = [
    "prompts_part1_tools.md",
    "prompts_part2_commands_hooks.md",
    "prompts_part3_services_skills.md",
    "prompts_part4_utils_query_constants.md",
    "prompts_part5_components_bridge_rest.md",
    "ALL_PROMPTS.md",
    "ALL_PROMPTS_PDF.md",
]

SECTION_START = re.compile(r"^### ", re.MULTILINE)
STATUS_LINE = re.compile(
    r"^\*\*Status:\s*(?:❌\s*)?NOT IN RUST\*\*(.*?)$", re.MULTILINE
)
CODE_BLOCK = re.compile(r"```(?:ts|typescript|tsx|js|javascript)?\n(.*?)```", re.DOTALL)


def _iter_rs_files():
    for p in CRATES.rglob("*.rs"):
        if any(part == "target" for part in p.parts):
            continue
        yield p


def _cache_rs_text() -> dict[Path, str]:
    cache: dict[Path, str] = {}
    for p in _iter_rs_files():
        try:
            cache[p] = p.read_text(encoding="utf-8")
        except Exception:
            pass
    return cache


def _pick_needles(ts_code: str) -> list[str]:
    """Pick up to ~6 distinctive substrings from the TS snippet
    to probe against the Rust tree. Returns multiple candidates;
    caller considers a hit if ANY needle matches.
    """
    cleaned = re.sub(r"\$\{[^}]+\}", "", ts_code)
    candidates: list[str] = []
    for m in re.finditer(r"`([^`\n][^`]{24,}?)`", cleaned):
        candidates.append(m.group(1))
    for m in re.finditer(r'"((?:[^"\\]|\\.){25,}?)"', cleaned):
        candidates.append(m.group(1).replace('\\"', '"'))
    for m in re.finditer(r"'((?:[^'\\]|\\.){25,}?)'", cleaned):
        candidates.append(m.group(1).replace("\\'", "'"))
    # Multi-line template literals: extract raw text runs between the backticks.
    for m in re.finditer(r"`([\s\S]{60,}?)`", cleaned):
        candidates.append(m.group(1))
    # Also grab any >=40-char run of printable text that looks like prose
    # (e.g. a prompt string broken across lines in TS without backticks).
    for m in re.finditer(r"[A-Z][a-zA-Z0-9 ,;:'`()\-\./_!?]{40,}", cleaned):
        candidates.append(m.group(0))
    # Deduplicate, sort by length desc, cap to 8 longest.
    seen: set[str] = set()
    uniq = []
    for c in sorted(candidates, key=len, reverse=True):
        clean = re.sub(r"\s+", " ", c).strip()
        if len(clean) < 30 or clean in seen:
            continue
        seen.add(clean)
        uniq.append(clean[:80])
        if len(uniq) >= 8:
            break
    return uniq


def _find_in_rs(needle: str, rs_cache: dict[Path, str]) -> list[Path]:
    """Return list of .rs paths whose content contains `needle`."""
    if not needle:
        return []
    hits: list[Path] = []
    # Normalize both needle and file text by collapsing whitespace.
    nnorm = re.sub(r"\s+", " ", needle).strip()
    for p, t in rs_cache.items():
        tnorm = re.sub(r"\s+", " ", t)
        if nnorm in tnorm:
            hits.append(p)
    return hits


def _process_doc(path: Path, rs_cache: dict[Path, str]) -> tuple[str, int, int]:
    """Return (new_text, stale_count, still_missing_count)."""
    text = path.read_text(encoding="utf-8")
    # Split into sections on `### ` headers.
    parts = SECTION_START.split(text)
    head = parts[0]
    sections = ["### " + p for p in parts[1:]]

    stale = 0
    missing = 0
    out = [head]
    for sec in sections:
        status_match = STATUS_LINE.search(sec)
        if not status_match:
            out.append(sec)
            continue
        # Extract the TS code block to pick needle candidates.
        cb = CODE_BLOCK.search(sec)
        needles = _pick_needles(cb.group(1)) if cb else []
        hits: list[Path] = []
        for needle in needles:
            h = _find_in_rs(needle, rs_cache)
            if h:
                hits = h
                break
        if hits:
            # Flip to ✅ FOUND in Rust with path references.
            rel_hits = sorted(
                {str(p.relative_to(ROOT)) for p in hits}
            )
            new_status = (
                "**Status: ✅ FOUND in Rust** — "
                + ", ".join(f"`{h}`" for h in rel_hits[:3])
                + (", …" if len(rel_hits) > 3 else "")
            )
            sec = STATUS_LINE.sub(new_status, sec, count=1)
            stale += 1
        else:
            missing += 1
        out.append(sec)
    return "".join(out), stale, missing


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--apply", action="store_true")
    args = ap.parse_args()

    rs_cache = _cache_rs_text()
    print(f"Scanning {len(rs_cache)} .rs files")
    total_stale = 0
    total_missing = 0
    for d in DOCS:
        p = ROOT / d
        if not p.exists():
            continue
        new_text, stale, missing = _process_doc(p, rs_cache)
        total_stale += stale
        total_missing += missing
        print(f"  {d}: stale={stale}, missing={missing}")
        if args.apply:
            p.write_text(new_text, encoding="utf-8")
    print(f"TOTAL: stale={total_stale}, missing={total_missing}")
    if not args.apply:
        print("(dry-run — rerun with --apply to write changes)")


if __name__ == "__main__":
    main()
