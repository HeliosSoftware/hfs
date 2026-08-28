#!/usr/bin/env python3
"""Static server for the DuckDB-WASM POC with correct MIME types.

Plain `python -m http.server` serves `.mjs` as `text/plain` on some platforms
(notably Windows Python), which browsers refuse to execute as ES modules. The
vendored Arrow ESM tree is `.mjs`, so serve it with the right types.

    python serve.py [port]      # default 8095, then open http://localhost:PORT/
"""
import http.server
import socketserver
import sys

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 8095


class Handler(http.server.SimpleHTTPRequestHandler):
    extensions_map = {
        **http.server.SimpleHTTPRequestHandler.extensions_map,
        ".js": "text/javascript",
        ".mjs": "text/javascript",
        ".wasm": "application/wasm",
        ".arrow": "application/octet-stream",
    }


with socketserver.TCPServer(("", PORT), Handler) as httpd:
    print(f"serving DuckDB-WASM POC on http://localhost:{PORT}/  (Ctrl+C to stop)")
    httpd.serve_forever()
