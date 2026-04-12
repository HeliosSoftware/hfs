#!/usr/bin/env python3
"""Minimal webhook capture server for subscriptions smoke tests."""

from __future__ import annotations

import argparse
import json
import threading
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path


def utc_now_iso() -> str:
    return datetime.now(timezone.utc).isoformat()


class CaptureState:
    def __init__(self, out_dir: Path) -> None:
        self.out_dir = out_dir
        self.out_dir.mkdir(parents=True, exist_ok=True)
        self._counter = 0
        self._lock = threading.Lock()

        self.requests_path = self.out_dir / "requests.ndjson"
        self.bodies_path = self.out_dir / "bodies.ndjson"

    def next_index(self) -> int:
        with self._lock:
            self._counter += 1
            return self._counter

    def append_json_line(self, path: Path, payload: dict) -> None:
        with path.open("a", encoding="utf-8") as handle:
            handle.write(json.dumps(payload, separators=(",", ":")))
            handle.write("\n")


class WebhookCaptureHandler(BaseHTTPRequestHandler):
    state: CaptureState

    def log_message(self, fmt: str, *args) -> None:
        # Keep CI logs focused; detailed request payloads are persisted to files.
        return

    def do_GET(self) -> None:
        if self.path != "/health":
            self.send_response(404)
            self.end_headers()
            return

        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(b'{"status":"ok"}')

    def do_POST(self) -> None:
        content_length = int(self.headers.get("Content-Length", "0"))
        body_raw = self.rfile.read(content_length)
        body_text = body_raw.decode("utf-8", errors="replace")

        try:
            parsed_body = json.loads(body_text)
        except json.JSONDecodeError:
            parsed_body = {"_raw_body": body_text}

        index = self.state.next_index()
        body_file = self.state.out_dir / f"request-{index:04d}.json"
        with body_file.open("w", encoding="utf-8") as handle:
            json.dump(parsed_body, handle, separators=(",", ":"))
            handle.write("\n")

        metadata = {
            "index": index,
            "timestamp": utc_now_iso(),
            "method": "POST",
            "path": self.path,
            "headers": {k: v for k, v in self.headers.items()},
            "body_file": body_file.name,
        }
        self.state.append_json_line(self.state.requests_path, metadata)
        self.state.append_json_line(self.state.bodies_path, parsed_body)

        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(b'{"ok":true}')


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Webhook capture server")
    parser.add_argument("--host", default="127.0.0.1", help="Bind host")
    parser.add_argument("--port", type=int, required=True, help="Bind port")
    parser.add_argument("--out-dir", required=True, help="Capture output directory")
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    out_dir = Path(args.out_dir).resolve()
    state = CaptureState(out_dir)

    WebhookCaptureHandler.state = state
    server = ThreadingHTTPServer((args.host, args.port), WebhookCaptureHandler)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
