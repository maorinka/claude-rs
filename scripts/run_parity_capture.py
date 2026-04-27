#!/usr/bin/env python3
"""Run installed Claude Code and claude-rs through the capture proxy.

This is the repeatable version of the manual TS/Rust proxy workflow:

  python3 scripts/run_parity_capture.py --prompt "Say hi."

It starts `context_capture_proxy.py`, runs both CLIs against it, then prints a
focused request-context diff. Captures are left on disk for deeper inspection.
"""

from __future__ import annotations

import argparse
import difflib
import json
import os
import pathlib
import shlex
import signal
import subprocess
import sys
import time
from typing import Any


ROOT = pathlib.Path(__file__).resolve().parents[1]
DEFAULT_OUTPUT_DIR = ROOT / "captures" / "context-proxy-live"


def run(cmd: list[str], env: dict[str, str], timeout: int) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        cmd,
        cwd=ROOT,
        env=env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=timeout,
        check=False,
    )


def wait_for_proxy(proc: subprocess.Popen[str], port: int) -> None:
    deadline = time.time() + 10
    while time.time() < deadline:
        if proc.poll() is not None:
            raise RuntimeError("context proxy exited before it was ready")
        try:
            import socket

            with socket.create_connection(("127.0.0.1", port), timeout=0.2):
                return
        except OSError:
            time.sleep(0.1)
    raise TimeoutError("context proxy did not start")


def load_capture(output_dir: pathlib.Path, marker: str) -> dict[str, Any]:
    matches = sorted(output_dir.glob(f"*{marker}*.json"))
    if not matches:
        raise FileNotFoundError(f"no capture matched {marker!r} in {output_dir}")
    return json.loads(matches[-1].read_text(encoding="utf-8"))


def block_types(blocks: Any) -> list[str]:
    if not isinstance(blocks, list):
        return []
    return [str(block.get("type", "?")) if isinstance(block, dict) else type(block).__name__ for block in blocks]


def message_shape(body: dict[str, Any]) -> list[str]:
    result = []
    for msg in body.get("messages", []):
        if not isinstance(msg, dict):
            result.append("?")
            continue
        result.append(f"{msg.get('role', '?')}:{','.join(block_types(msg.get('content')))}")
    return result


def tool_names(body: dict[str, Any]) -> list[str]:
    return [
        str(tool.get("name", "?"))
        for tool in body.get("tools", [])
        if isinstance(tool, dict)
    ]


def skill_names(body: dict[str, Any]) -> list[str]:
    for msg in body.get("messages", []):
        for block in msg.get("content", []) if isinstance(msg, dict) else []:
            text = block.get("text", "") if isinstance(block, dict) else ""
            if "The following skills are available" not in text:
                continue
            return [
                line[2:].split(": ", 1)[0]
                for line in text.splitlines()
                if line.startswith("- ")
            ]
    return []


def scrub_body(body: dict[str, Any]) -> dict[str, Any]:
    def scrub(value: Any) -> Any:
        if isinstance(value, dict):
            return {
                key: scrub(val)
                for key, val in value.items()
                if key not in {"cache_control", "metadata"}
            }
        if isinstance(value, list):
            return [scrub(item) for item in value]
        return value

    result = scrub(body)
    if isinstance(result, dict) and isinstance(result.get("system"), list) and result["system"]:
        if isinstance(result["system"][0], dict):
            result["system"][0]["text"] = "<dynamic-system-block-0>"
    return result


def important_headers(capture: dict[str, Any]) -> dict[str, str]:
    headers = capture.get("headers", {})
    out = {}
    for wanted in ["anthropic-beta", "user-agent", "x-stainless-lang"]:
        for key, value in headers.items():
            if key.lower() == wanted:
                out[wanted] = value
                break
    return out


def print_report(ts: dict[str, Any], rs: dict[str, Any]) -> None:
    ts_body = ts["body"]
    rs_body = rs["body"]
    print("== Request summary ==")
    print(f"TS: {ts.get('summary')}")
    print(f"RS: {rs.get('summary')}")
    print()

    print("== Headers ==")
    print(f"TS: {important_headers(ts)}")
    print(f"RS: {important_headers(rs)}")
    print()

    print("== Body keys ==")
    for key in ["model", "max_tokens", "stream", "thinking", "context_management", "output_config"]:
        marker = "==" if ts_body.get(key) == rs_body.get(key) else "!="
        print(f"{key}: {marker}")
    print(f"message_shape: {'==' if message_shape(ts_body) == message_shape(rs_body) else '!='}")
    print()

    ts_tools = tool_names(ts_body)
    rs_tools = tool_names(rs_body)
    print("== Tools ==")
    print(f"count: TS {len(ts_tools)} / RS {len(rs_tools)}")
    print(f"order: {'==' if ts_tools == rs_tools else '!='}")
    missing = sorted(set(ts_tools) - set(rs_tools))
    extra = sorted(set(rs_tools) - set(ts_tools))
    if missing:
        print(f"missing in RS: {missing}")
    if extra:
        print(f"extra in RS: {extra}")
    print()

    ts_skills = skill_names(ts_body)
    rs_skills = skill_names(rs_body)
    print("== Skills ==")
    print(f"count: TS {len(ts_skills)} / RS {len(rs_skills)}")
    print(f"set: {'==' if set(ts_skills) == set(rs_skills) else '!='}")
    print(f"order: {'==' if ts_skills == rs_skills else '!='}")
    if ts_skills != rs_skills:
        for line in difflib.unified_diff(ts_skills, rs_skills, fromfile="ts", tofile="rs", lineterm=""):
            print(line)
    print()

    ts_scrubbed = scrub_body(ts_body)
    rs_scrubbed = scrub_body(rs_body)
    print("== Scrubbed body ==")
    print(f"equal: {'yes' if ts_scrubbed == rs_scrubbed else 'no'}")


def stdout_events(text: str) -> list[str]:
    events = []
    for line in text.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            event = json.loads(line)
        except Exception:
            events.append(f"text:{line[:80]}")
            continue
        if not isinstance(event, dict):
            events.append(type(event).__name__)
            continue
        parts = [str(event.get("type", "?"))]
        if event.get("subtype"):
            parts.append(str(event["subtype"]))
        if event.get("name"):
            parts.append(str(event["name"]))
        if event.get("tool_name"):
            parts.append(str(event["tool_name"]))
        events.append("/".join(parts))
    return events


def print_stdout_report(ts_stdout: str, rs_stdout: str) -> None:
    ts_events = stdout_events(ts_stdout)
    rs_events = stdout_events(rs_stdout)
    print("== Stdout ==")
    print(f"event shape: {'==' if ts_events == rs_events else '!='}")
    print(f"TS events: {ts_events}")
    print(f"RS events: {rs_events}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--prompt", default="Say hi.")
    parser.add_argument("--port", type=int, default=8787)
    parser.add_argument("--output-dir", type=pathlib.Path, default=DEFAULT_OUTPUT_DIR)
    parser.add_argument("--timeout", type=int, default=180)
    parser.add_argument("--max-turns", default="1")
    parser.add_argument("--output-format", choices=["text", "json", "stream-json"], default="text")
    parser.add_argument("--ts-command", default="claude")
    parser.add_argument("--rust-command", default="cargo run -q -p claude-cli --")
    parser.add_argument("--no-clean", action="store_true", help="append to output dir instead of recreating it")
    args = parser.parse_args()

    if not args.no_clean and args.output_dir.exists():
        import shutil

        shutil.rmtree(args.output_dir)
    args.output_dir.mkdir(parents=True, exist_ok=True)

    proxy_cmd = [
        sys.executable,
        str(ROOT / "scripts" / "context_capture_proxy.py"),
        "--port",
        str(args.port),
        "--output-dir",
        str(args.output_dir),
    ]
    proxy = subprocess.Popen(
        proxy_cmd,
        cwd=ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    try:
        wait_for_proxy(proxy, args.port)
        base_env = os.environ.copy()
        ts_env = base_env | {"ANTHROPIC_BASE_URL": f"http://127.0.0.1:{args.port}"}
        rs_env = base_env | {
            "CLAUDE_DEBUG_PROXY_BASE": f"http://127.0.0.1:{args.port}",
            "RUSTC_WRAPPER": "",
        }

        ts_cmd = shlex.split(args.ts_command) + [
            "-p",
            args.prompt,
            "--max-turns",
            args.max_turns,
            "--dangerously-skip-permissions",
        ]
        if args.output_format != "text":
            ts_cmd.extend(["--output-format", args.output_format])
        rs_cmd = shlex.split(args.rust_command) + [
            "--print",
            args.prompt,
            "--max-turns",
            args.max_turns,
            "--dangerously-skip-permissions",
        ]
        if args.output_format != "text":
            rs_cmd.extend(["--output-format", args.output_format])

        print(f"running TS: {' '.join(shlex.quote(p) for p in ts_cmd)}")
        ts_result = run(ts_cmd, ts_env, args.timeout)
        print(ts_result.stdout.strip())
        if ts_result.returncode != 0:
            print(ts_result.stderr, file=sys.stderr)
            return ts_result.returncode

        print(f"running RS: {' '.join(shlex.quote(p) for p in rs_cmd)}")
        rs_result = run(rs_cmd, rs_env, args.timeout)
        print(rs_result.stdout.strip())
        if rs_result.returncode != 0:
            print(rs_result.stderr, file=sys.stderr)
            return rs_result.returncode
    finally:
        if proxy.poll() is None:
            proxy.send_signal(signal.SIGINT)
            try:
                proxy.communicate(timeout=5)
            except subprocess.TimeoutExpired:
                proxy.kill()
                proxy.communicate()

    ts_capture = load_capture(args.output_dir, "-ts-v1-messages")
    rs_capture = load_capture(args.output_dir, "-rs-api-v1-messages")
    print()
    print_report(ts_capture, rs_capture)
    if args.output_format != "text":
        print()
        print_stdout_report(ts_result.stdout, rs_result.stdout)
    print()
    print(f"captures: {args.output_dir}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
