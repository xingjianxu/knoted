"""Loopback-only synthetic model for the T71 isolated instance."""

import hmac
import json
import re
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

TOKEN = Path(sys.argv[1]).read_text(encoding="utf-8").strip()
MODEL = "t71-fixture"
PORT = int(sys.argv[2])

class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def authorize(self):
        if hmac.compare_digest(self.headers.get("Authorization", ""), "Bearer " + TOKEN):
            return True
        self.send_error(401)
        return False

    def json_response(self, body):
        data = json.dumps(body, ensure_ascii=False).encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        if not self.authorize():
            return
        if self.path == "/v1/status":
            self.json_response({
                "service": "strata", "model": MODEL, "loaded": True, "cache_max_tokens": 32768,
                "context": {"native": 32768, "max_positions": 32768},
                "activity": {"in_flight": 0},
            })
        elif self.path == "/v1/models":
            self.json_response({"data": [{"id": MODEL, "meta": {"n_ctx": 32768}}]})
        else:
            self.send_error(404)

    def do_POST(self):
        if not self.authorize():
            return
        if self.path != "/v1/chat/completions":
            self.send_error(404)
            return
        try:
            size = int(self.headers["Content-Length"])
            if not 0 < size <= 2_000_000:
                raise ValueError
            request = json.loads(self.rfile.read(size))
            if request["model"] != MODEL or request["max_tokens"] != 4096 or not request["stream"]:
                raise ValueError
            messages = request["messages"]
            if messages[-1]["content"] != "violetamber":
                raise ValueError
            sources = [json.loads(item["content"].removeprefix("检索资料："))
                       for item in messages[:-1]
                       if item["role"] == "user" and item["content"].startswith("检索资料：")]
            fixture = next(s for s in sources if "violetamber" in s["text"] and "紫檀" in s["text"])
            marker = fixture["citation"]
            if not re.fullmatch(r"\[cite:[0-9a-f-]{36}\]", marker):
                raise ValueError
        except (ValueError, KeyError, StopIteration, TypeError, IndexError):
            # Keep prompts, source text and credentials out of diagnostics.
            self.send_error(422)
            return

        frames = [
            {"model": MODEL, "choices": [{"index": 0, "delta": {
                "content": f"观测资料记载了紫檀标本。 {marker}"}, "finish_reason": None}]},
            {"model": MODEL, "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]},
        ]
        body = ("".join("data: " + json.dumps(frame, ensure_ascii=False) + "\n\n" for frame in frames)
                + "data: [DONE]\n\n").encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
