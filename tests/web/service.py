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
import socketserver
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


class Server(http.server.ThreadingHTTPServer):
    """Binds without asking DNS who we are.

    `HTTPServer.server_bind` calls `socket.getfqdn()`, a reverse lookup that can stall for
    tens of seconds where no resolver answers — a CI runner, typically. The socket is bound by
    then, so the port looks open while nothing is accepting yet, and every request queues in
    the backlog until it times out. Skipping the lookup is the whole fix.
    """

    def server_bind(self):
        socketserver.TCPServer.server_bind(self)
        self.server_name = "127.0.0.1"
        self.server_port = self.server_address[1]


server = Server(("127.0.0.1", PORT), Service)
print(f"listening on {PORT}", flush=True)
server.serve_forever()
