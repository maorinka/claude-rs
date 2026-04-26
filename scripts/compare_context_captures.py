#!/usr/bin/env python3
"""Compare two captured Claude request bodies."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


def load(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def block_types(blocks: Any) -> list[str]:
    if not isinstance(blocks, list):
        return []
    result = []
    for block in blocks:
        if isinstance(block, dict):
            result.append(str(block.get("type", "?")))
        else:
            result.append(type(block).__name__)
    return result


def message_shape(body: dict[str, Any]) -> list[str]:
    messages = body.get("messages")
    if not isinstance(messages, list):
        return []
    shape = []
    for msg in messages:
        if not isinstance(msg, dict):
            shape.append("?")
            continue
        role = msg.get("role", "?")
        content = msg.get("content")
        shape.append(f"{role}:{','.join(block_types(content))}")
    return shape


def compare(label_a: str, a: dict[str, Any], label_b: str, b: dict[str, Any]) -> None:
    body_a = a["body"]
    body_b = b["body"]

    print(f"{label_a}: {a.get('summary')}")
    print(f"{label_b}: {b.get('summary')}")
    if a.get("response") or b.get("response"):
        print(f"{label_a} response usage: {a.get('response', {}).get('usage_events')}")
        print(f"{label_b} response usage: {b.get('response', {}).get('usage_events')}")
    print()

    for key in ["model", "max_tokens", "stream", "thinking", "context_management", "metadata"]:
        va = body_a.get(key)
        vb = body_b.get(key)
        marker = "==" if va == vb else "!="
        print(f"{key}: {marker}")
        if va != vb:
            print(f"  {label_a}: {json.dumps(va, ensure_ascii=False)[:500]}")
            print(f"  {label_b}: {json.dumps(vb, ensure_ascii=False)[:500]}")

    print()
    shape_a = message_shape(body_a)
    shape_b = message_shape(body_b)
    print(f"messages shape: {'==' if shape_a == shape_b else '!='}")
    print(f"  {label_a}: {shape_a}")
    print(f"  {label_b}: {shape_b}")

    print()
    print(f"system block types:")
    print(f"  {label_a}: {block_types(body_a.get('system'))}")
    print(f"  {label_b}: {block_types(body_b.get('system'))}")
    print(f"tool names:")
    print(f"  {label_a}: {tool_names(body_a)}")
    print(f"  {label_b}: {tool_names(body_b)}")


def tool_names(body: dict[str, Any]) -> list[str]:
    tools = body.get("tools")
    if not isinstance(tools, list):
        return []
    return [str(t.get("name", "?")) for t in tools if isinstance(t, dict)]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("first", type=Path)
    parser.add_argument("second", type=Path)
    parser.add_argument("--first-label", default="first")
    parser.add_argument("--second-label", default="second")
    args = parser.parse_args()
    compare(args.first_label, load(args.first), args.second_label, load(args.second))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
