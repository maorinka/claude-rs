#!/usr/bin/env python3
"""Capture and relay Claude API request contexts for Rust/TS comparison.

Usage:
  python3 scripts/context_capture_proxy.py --port 8787

Rust:
  CLAUDE_DEBUG_PROXY_BASE=http://127.0.0.1:8787 cargo run -p claude-cli -- ...

Installed TS/current Claude:
  ANTHROPIC_BASE_URL=http://127.0.0.1:8787 claude

Captured requests are written to captures/context-proxy/*.json.
"""

from __future__ import annotations

import argparse
import datetime as dt
import http.client
import json
import pathlib
import re
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any
from urllib.parse import urlsplit


DEFAULT_UPSTREAM = "https://api.anthropic.com"
DEFAULT_OUTPUT_DIR = pathlib.Path("captures/context-proxy")
SENSITIVE_HEADERS = {
    "authorization",
    "x-api-key",
    "anthropic-api-key",
    "cookie",
    "set-cookie",
}


def sanitize_headers(headers: BaseHTTPRequestHandler.headers) -> dict[str, str]:
    result: dict[str, str] = {}
    for key, value in headers.items():
        if key.lower() in SENSITIVE_HEADERS:
            result[key] = "<redacted>"
        else:
            result[key] = value
    return result


def safe_slug(value: str) -> str:
    value = value.strip().lower() or "unknown"
    value = re.sub(r"[^a-z0-9_.-]+", "-", value)
    return value.strip("-") or "unknown"


def summarize_body(body: Any) -> dict[str, Any]:
    if not isinstance(body, dict):
        return {"body_type": type(body).__name__}
    messages = body.get("messages")
    system = body.get("system")
    tools = body.get("tools")
    return {
        "model": body.get("model"),
        "max_tokens": body.get("max_tokens"),
        "message_count": len(messages) if isinstance(messages, list) else None,
        "system_blocks": len(system) if isinstance(system, list) else None,
        "tool_count": len(tools) if isinstance(tools, list) else None,
        "has_thinking": "thinking" in body,
        "has_context_management": "context_management" in body,
        "cache_markers": count_cache_markers(body),
    }


def count_cache_markers(value: Any) -> int:
    if isinstance(value, dict):
        count = 1 if value.get("cache_control") else 0
        return count + sum(count_cache_markers(v) for v in value.values())
    if isinstance(value, list):
        return sum(count_cache_markers(v) for v in value)
    return 0


def extract_sse_usage_events(text: str) -> list[dict[str, Any]]:
    usage_events: list[dict[str, Any]] = []
    for event_text in text.split("\n\n"):
        data_lines = [
            line.removeprefix("data:").strip()
            for line in event_text.splitlines()
            if line.startswith("data:")
        ]
        if not data_lines:
            continue
        data = "\n".join(data_lines)
        if data == "[DONE]":
            continue
        try:
            event = json.loads(data)
        except Exception:
            continue
        usage = event.get("usage")
        if isinstance(usage, dict):
            usage_events.append(
                {
                    "event_type": event.get("type"),
                    "usage": usage,
                }
            )
        message = event.get("message")
        if isinstance(message, dict) and isinstance(message.get("usage"), dict):
            usage_events.append(
                {
                    "event_type": event.get("type"),
                    "usage": message["usage"],
                }
            )
    return usage_events


class CaptureProxy(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    upstream_base = DEFAULT_UPSTREAM
    output_dir = DEFAULT_OUTPUT_DIR

    def do_POST(self) -> None:
        raw_body = self.rfile.read(int(self.headers.get("content-length", "0") or "0"))
        client_tag = self.headers.get("x-client-tag") or self.headers.get("x-context-client")
        if not client_tag:
            client_tag = "RS" if self.path.startswith("/api/") else "TS"

        body_json: Any
        try:
            body_json = json.loads(raw_body.decode("utf-8"))
        except Exception:
            body_json = {"_raw_body": raw_body.decode("utf-8", errors="replace")}

        capture_path = self.capture_request(client_tag, body_json)
        self.relay_request(raw_body, capture_path)

    def capture_request(self, client_tag: str, body_json: Any) -> pathlib.Path:
        self.output_dir.mkdir(parents=True, exist_ok=True)
        stamp = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%S.%fZ")
        path_slug = safe_slug(self.path)
        tag_slug = safe_slug(client_tag)
        out_path = self.output_dir / f"{stamp}-{tag_slug}-{path_slug}.json"
        payload = {
            "captured_at": stamp,
            "client_tag": client_tag,
            "method": self.command,
            "path": self.path,
            "headers": sanitize_headers(self.headers),
            "summary": summarize_body(body_json),
            "body": body_json,
        }
        out_path.write_text(json.dumps(payload, indent=2, sort_keys=True), encoding="utf-8")
        print(
            f"captured {client_tag} {self.path} -> {out_path} "
            f"{payload['summary']}",
            flush=True,
        )
        return out_path

    def relay_request(self, raw_body: bytes, capture_path: pathlib.Path) -> None:
        path = self.path
        if path.startswith("/api/"):
            upstream = urlsplit(DEFAULT_UPSTREAM)
            path = path[len("/api") :]
        elif path.startswith("/platform/"):
            upstream = urlsplit("https://platform.claude.com")
            path = path[len("/platform") :]
        elif path.startswith("/cai/"):
            upstream = urlsplit("https://claude.com")
            path = "/cai" + path[len("/cai") :]
        else:
            upstream = urlsplit(self.upstream_base)

        scheme = upstream.scheme or "https"
        host = upstream.netloc or upstream.path

        conn_cls = http.client.HTTPSConnection if scheme == "https" else http.client.HTTPConnection
        conn = conn_cls(host, timeout=300)
        headers = {
            key: value
            for key, value in self.headers.items()
            if key.lower()
            not in {
                "host",
                "content-length",
                "connection",
                "accept-encoding",
                "x-client-tag",
                "x-context-client",
            }
        }
        headers["Host"] = host
        headers["Content-Length"] = str(len(raw_body))

        try:
            conn.request(self.command, path, body=raw_body, headers=headers)
            resp = conn.getresponse()
            self.send_response(resp.status, resp.reason)
            for key, value in resp.getheaders():
                if key.lower() in {"connection", "transfer-encoding", "content-length"}:
                    continue
                self.send_header(key, value)
            self.end_headers()
            response_body = bytearray()
            while True:
                chunk = resp.read(64 * 1024)
                if not chunk:
                    break
                response_body.extend(chunk)
                self.wfile.write(chunk)
                self.wfile.flush()
            self.capture_response(capture_path, resp.status, bytes(response_body))
        except Exception as exc:
            self.send_error(502, f"proxy relay failed: {exc}")
        finally:
            conn.close()

    def capture_response(self, capture_path: pathlib.Path, status: int, raw_body: bytes) -> None:
        try:
            payload = json.loads(capture_path.read_text(encoding="utf-8"))
        except Exception:
            return
        text = raw_body.decode("utf-8", errors="replace")
        usage_events = extract_sse_usage_events(text)
        payload["response"] = {
            "status": status,
            "usage_events": usage_events,
            "final_usage": usage_events[-1] if usage_events else None,
            "raw_preview": text[:20_000],
        }
        capture_path.write_text(
            json.dumps(payload, indent=2, sort_keys=True),
            encoding="utf-8",
        )

    def log_message(self, fmt: str, *args: Any) -> None:
        print(f"{self.address_string()} - {fmt % args}", file=sys.stderr)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8787)
    parser.add_argument("--upstream", default=DEFAULT_UPSTREAM)
    parser.add_argument("--output-dir", type=pathlib.Path, default=DEFAULT_OUTPUT_DIR)
    args = parser.parse_args()

    CaptureProxy.upstream_base = args.upstream.rstrip("/")
    CaptureProxy.output_dir = args.output_dir

    server = ThreadingHTTPServer((args.host, args.port), CaptureProxy)
    print(
        f"context capture proxy listening on http://{args.host}:{args.port} "
        f"-> {CaptureProxy.upstream_base}",
        flush=True,
    )
    server.serve_forever()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
