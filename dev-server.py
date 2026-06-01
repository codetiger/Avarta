#!/usr/bin/env python3
"""Static dev server for web/ that disables caching.

Browsers aggressively cache the .wasm / ES modules, so after a `wasm-pack`
rebuild a normal reload can keep running the OLD wasm (which silently ignores
any newly added parameters). `no-store` forces a fresh fetch every reload.

Usage:  python3 dev-server.py [port]   # default 8080
"""
import functools
import http.server
import socketserver
import sys

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 8080


class NoCacheHandler(http.server.SimpleHTTPRequestHandler):
    def end_headers(self):
        self.send_header("Cache-Control", "no-store, max-age=0")
        super().end_headers()


NoCacheHandler.extensions_map[".wasm"] = "application/wasm"
handler = functools.partial(NoCacheHandler, directory="web")

with socketserver.TCPServer(("", PORT), handler) as httpd:
    print(f"serving web/ on http://localhost:{PORT}  (Cache-Control: no-store)")
    httpd.serve_forever()
