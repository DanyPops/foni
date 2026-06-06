#!/usr/bin/env python3.11
"""
Ruaccent HTTP sidecar — listens on :8765, annotates Russian text with stress.

POST /annotate
  Body: {"text": "..."}
  Response: {"text": "..."}   (Unicode combining acute U+0301 after stressed vowel)

Ruaccent uses '+' before the stressed vowel; we convert to Unicode on output.
"""

import re
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer
import json
import warnings

warnings.filterwarnings("ignore")

PORT = 8765
VOWELS = set("аеёиоуыэюяАЕЁИОУЫЭЮЯ")

print("[ruaccent] loading model...", flush=True)
from ruaccent import RUAccent  # noqa: E402

acc = RUAccent()
acc.load(omograph_model_size="turbo", use_dictionary=True)
print("[ruaccent] ready", flush=True)


def plus_to_unicode(text: str) -> str:
    """Convert ruaccent '+' notation to Unicode combining acute (U+0301).

    Input:  'з+амок'
    Output: 'за\u0301мок'  (= за́мок)
    """
    result = []
    i = 0
    while i < len(text):
        ch = text[i]
        if ch == "+" and i + 1 < len(text) and text[i + 1] in VOWELS:
            # Skip the '+', emit the vowel, then the combining acute.
            result.append(text[i + 1])
            result.append("\u0301")
            i += 2
        else:
            result.append(ch)
            i += 1
    return "".join(result)


class Handler(BaseHTTPRequestHandler):
    def log_message(self, fmt, *args):
        pass  # suppress access log noise

    def do_POST(self):
        if self.path != "/annotate":
            self.send_response(404)
            self.end_headers()
            return

        length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(length)
        try:
            payload = json.loads(body)
            text = payload.get("text", "")
            annotated = plus_to_unicode(acc.process_all(text))
            response = json.dumps({"text": annotated}).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(response)))
            self.end_headers()
            self.wfile.write(response)
        except Exception as e:
            err = json.dumps({"error": str(e)}).encode()
            self.send_response(500)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(err)))
            self.end_headers()
            self.wfile.write(err)


if __name__ == "__main__":
    server = HTTPServer(("127.0.0.1", PORT), Handler)
    print(f"[ruaccent] listening on http://127.0.0.1:{PORT}", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("[ruaccent] stopped", flush=True)
        sys.exit(0)
