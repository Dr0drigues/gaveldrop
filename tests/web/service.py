"""A minimal order service, standing in for a real one under test.

Python because it is on both CI images already, so a fixture never fails for a runtime that
was not installed. It answers on the port gaveldrop reserved, and reaches its own dependency
at the faked one.

Threading, because the single-threaded server answers one connection at a time with a small
accept backlog: on a slow machine the readiness probes queue up behind each other and the
service looks dead while it is listening.
"""

import http.server
import json
import os
import pathlib
import urllib.error
import urllib.request

PORT = int(os.environ["GAVELDROP_PORT"])
FAKE = os.environ.get("GAVELDROP_FAKE_PORT", "0")
HOME = pathlib.Path(os.environ["HOME"])


class Service(http.server.BaseHTTPRequestHandler):
    def log_message(self, fmt, *args):
        print(fmt % args, flush=True)

    def answer(self, status, payload, content_type="application/json"):
        body = json.dumps(payload).encode() if content_type.endswith("json") else payload.encode()
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path == "/health":
            self.answer(200, {"status": "ok"})
        elif self.path == "/catalogue":
            self.answer(200, self.from_upstream())
        else:
            self.answer(404, {"error": "no such thing"})

    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        asked = json.loads(self.rfile.read(length) or b"{}")
        (HOME / "orders.log").write_text(f"created {asked.get('item', 'nothing')}\n")
        self.answer(201, {"id": 7, "item": asked.get("item")})

    def from_upstream(self):
        """Calls the dependency, which gaveldrop replaces with the fake."""
        try:
            with urllib.request.urlopen(f"http://127.0.0.1:{FAKE}/products", timeout=5) as answer:
                return {"upstream": json.loads(answer.read())}
        except (urllib.error.URLError, json.JSONDecodeError) as failure:
            return {"upstream_error": str(failure)}


print(f"listening on {PORT}", flush=True)
http.server.ThreadingHTTPServer(("127.0.0.1", PORT), Service).serve_forever()
