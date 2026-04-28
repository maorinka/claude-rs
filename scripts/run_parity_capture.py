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
import tempfile
import time
from typing import Any


ROOT = pathlib.Path(__file__).resolve().parents[1]
DEFAULT_OUTPUT_DIR = pathlib.Path(tempfile.gettempdir()) / "claude-rs-context-proxy-live"


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


def normalize_skill_line_order(body: dict[str, Any]) -> dict[str, Any]:
    result = json.loads(json.dumps(body))
    for msg in result.get("messages", []):
        for block in msg.get("content", []) if isinstance(msg, dict) else []:
            text = block.get("text", "") if isinstance(block, dict) else ""
            if "The following skills are available" not in text:
                continue
            lines = text.splitlines()
            skill_indexes = [i for i, line in enumerate(lines) if line.startswith("- ")]
            if not skill_indexes:
                continue
            first = skill_indexes[0]
            last = skill_indexes[-1]
            block["text"] = "\n".join(
                lines[:first] + sorted(lines[i] for i in skill_indexes) + lines[last + 1 :]
            )
    return result


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
        for block in result["system"]:
            if not isinstance(block, dict):
                continue
            text = block.get("text")
            if isinstance(text, str) and text.startswith("x-anthropic-billing-header:"):
                block["text"] = "<dynamic-billing-header>"
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
    ts_normalized = normalize_skill_line_order(ts_scrubbed)
    rs_normalized = normalize_skill_line_order(rs_scrubbed)
    print(f"equal ignoring skill order: {'yes' if ts_normalized == rs_normalized else 'no'}")


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


def stdout_json_events(text: str) -> list[dict[str, Any]]:
    events = []
    for line in text.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            event = json.loads(line)
        except Exception:
            continue
        if isinstance(event, dict):
            events.append(event)
    return events


def first_event(events: list[dict[str, Any]], type_: str, subtype: str | None = None) -> dict[str, Any] | None:
    for event in events:
        if event.get("type") != type_:
            continue
        if subtype is not None and event.get("subtype") != subtype:
            continue
        return event
    return None


def print_array_field_diff(label: str, ts_event: dict[str, Any], rs_event: dict[str, Any]) -> None:
    ts_values = ts_event.get(label) or []
    rs_values = rs_event.get(label) or []
    if not isinstance(ts_values, list) or not isinstance(rs_values, list):
        print(f"{label}: non-list")
        return
    print(f"{label}: count TS {len(ts_values)} / RS {len(rs_values)}, set {'==' if set(map(str, ts_values)) == set(map(str, rs_values)) else '!='}, order {'==' if ts_values == rs_values else '!='}")
    if ts_values != rs_values:
        for line in difflib.unified_diff(
            [str(value) for value in ts_values],
            [str(value) for value in rs_values],
            fromfile=f"ts:{label}",
            tofile=f"rs:{label}",
            lineterm="",
        ):
            print(line)


def sorted_keys(value: Any) -> list[str]:
    return sorted(value.keys()) if isinstance(value, dict) else []


def nested(value: dict[str, Any] | None, *path: str) -> Any:
    current: Any = value
    for part in path:
        if not isinstance(current, dict):
            return None
        current = current.get(part)
    return current


def scrub_init_scalar(label: str, value: Any) -> Any:
    if label in {"session_id", "uuid", "cwd"}:
        return "<dynamic>"
    return value


def print_stdout_report(ts_stdout: str, rs_stdout: str) -> None:
    ts_events = stdout_events(ts_stdout)
    rs_events = stdout_events(rs_stdout)
    print("== Stdout ==")
    print(f"event shape: {'==' if ts_events == rs_events else '!='}")
    print(f"TS events: {ts_events}")
    print(f"RS events: {rs_events}")
    ts_init = first_event(stdout_json_events(ts_stdout), "system", "init")
    rs_init = first_event(stdout_json_events(rs_stdout), "system", "init")
    if ts_init and rs_init:
        print()
        print("== Stdout system/init ==")
        ts_keys = sorted(ts_init.keys())
        rs_keys = sorted(rs_init.keys())
        print(f"keys: {'==' if ts_keys == rs_keys else '!='}")
        if ts_keys != rs_keys:
            print(f"missing in RS: {sorted(set(ts_keys) - set(rs_keys))}")
            print(f"extra in RS: {sorted(set(rs_keys) - set(ts_keys))}")
        for field in [
            "cwd",
            "session_id",
            "model",
            "permissionMode",
            "apiKeySource",
            "claude_code_version",
            "output_style",
            "analytics_disabled",
            "fast_mode_state",
            "memory_paths",
            "betas",
        ]:
            if field not in ts_init and field not in rs_init:
                continue
            ts_value = scrub_init_scalar(field, ts_init.get(field))
            rs_value = scrub_init_scalar(field, rs_init.get(field))
            print(f"{field}: {'==' if ts_value == rs_value else '!='}")
        for field in ["tools", "slash_commands", "skills", "agents"]:
            print_array_field_diff(field, ts_init, rs_init)

    ts_assistant = first_event(stdout_json_events(ts_stdout), "assistant")
    rs_assistant = first_event(stdout_json_events(rs_stdout), "assistant")
    if ts_assistant and rs_assistant:
        print()
        print("== Stdout assistant ==")
        for label, ts_value, rs_value in [
            ("event keys", ts_assistant, rs_assistant),
            ("message keys", nested(ts_assistant, "message"), nested(rs_assistant, "message")),
            (
                "usage keys",
                nested(ts_assistant, "message", "usage"),
                nested(rs_assistant, "message", "usage"),
            ),
        ]:
            print(f"{label}: {'==' if sorted_keys(ts_value) == sorted_keys(rs_value) else '!='}")

    ts_result = first_event(stdout_json_events(ts_stdout), "result", "success")
    rs_result = first_event(stdout_json_events(rs_stdout), "result", "success")
    if ts_result and rs_result:
        print()
        print("== Stdout result ==")
        for label, ts_value, rs_value in [
            ("event keys", ts_result, rs_result),
            ("usage keys", nested(ts_result, "usage"), nested(rs_result, "usage")),
            (
                "iteration keys",
                (nested(ts_result, "usage", "iterations") or [{}])[0],
                (nested(rs_result, "usage", "iterations") or [{}])[0],
            ),
            (
                "model usage keys",
                next(iter((ts_result.get("modelUsage") or {}).values()), {}),
                next(iter((rs_result.get("modelUsage") or {}).values()), {}),
            ),
        ]:
            print(f"{label}: {'==' if sorted_keys(ts_value) == sorted_keys(rs_value) else '!='}")


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
    parser.add_argument("--keep-going-on-error", action="store_true", help="run both CLIs and print a diff even if one exits nonzero")
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
        (args.output_dir / "ts.stdout").write_text(ts_result.stdout, encoding="utf-8")
        (args.output_dir / "ts.stderr").write_text(ts_result.stderr, encoding="utf-8")
        print(ts_result.stdout.strip())
        if ts_result.returncode != 0:
            print(ts_result.stderr, file=sys.stderr)
            if not args.keep_going_on_error:
                return ts_result.returncode

        print(f"running RS: {' '.join(shlex.quote(p) for p in rs_cmd)}")
        rs_result = run(rs_cmd, rs_env, args.timeout)
        (args.output_dir / "rs.stdout").write_text(rs_result.stdout, encoding="utf-8")
        (args.output_dir / "rs.stderr").write_text(rs_result.stderr, encoding="utf-8")
        print(rs_result.stdout.strip())
        if rs_result.returncode != 0:
            print(rs_result.stderr, file=sys.stderr)
            if not args.keep_going_on_error:
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
