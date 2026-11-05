#!/usr/bin/env python3
"""Browser behavior test; run with python3 tests/dashboard-project-rows.py (requires Chromium)."""
import http.server
import json
import pathlib
import queue
import subprocess
import tempfile
import threading
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1] / "dashboard"
RESULTS = queue.Queue()
RUNS = [
    {"id": "1", "title": "Z running", "project": "/work/zeta", "stage": "build", "status": "running", "updated_at": 200},
    {"id": "2", "title": "A running", "project": "/work/alpha", "stage": "build", "status": "running", "updated_at": 300},
    {"id": "3", "title": "Z waiting", "project": "/work/zeta", "stage": "build", "status": "blocked", "updated_at": 100},
    {"id": "4", "title": "A define", "project": "/work/alpha", "stage": "define", "status": "running", "updated_at": 400},
]
CHECK = """
<script>
const check = setInterval(() => {
  const rows = [...document.querySelectorAll('.project-row')];
  if (rows.length !== 2) return;
  clearInterval(check);
  const names = rows.map(row => row.querySelector('.project-heading').textContent.trim());
  const builds = rows.map(row => [...row.querySelector('[aria-label="Build"]').querySelectorAll('.run h3')].map(h => h.textContent.trim()));
  fetch("/result", {method: "POST", body: JSON.stringify({names, builds})});
}, 10);
</script>
"""

class Handler(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        if self.path != "/result":
            self.send_error(404)
            return
        data = self.rfile.read(int(self.headers["Content-Length"]))
        RESULTS.put(json.loads(data))
        self.send_response(204)
        self.end_headers()

    def do_GET(self):
        if self.path == "/api/runs":
            data, kind = json.dumps(RUNS).encode(), "application/json"
        elif self.path == "/":
            data, kind = (ROOT / "index.html").read_bytes() + CHECK.encode(), "text/html"
        elif self.path.startswith("/assets/"):
            path = ROOT / self.path.removeprefix("/assets/")
            data = path.read_bytes()
            kind = "text/javascript" if path.suffix == ".js" else "text/css"
        else:
            self.send_error(404)
            return
        self.send_response(200)
        self.send_header("Content-Type", kind)
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *args):
        pass

class ProjectRowsTest(unittest.TestCase):
    def test_project_rows_and_waiting_order(self):
        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        thread = threading.Thread(target=server.serve_forever)
        thread.start()
        try:
            with tempfile.TemporaryDirectory() as profile:
                process = subprocess.Popen(
                    ["chromium", "--headless", "--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage",
                     "--user-data-dir=" + profile, f"http://127.0.0.1:{server.server_port}/"],
                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                )
                try:
                    result = RESULTS.get(timeout=30)
                finally:
                    process.terminate()
                    process.wait(timeout=10)
            self.assertEqual(result["names"], ["alpha", "zeta"])
            self.assertEqual(result["builds"], [["A running"], ["Z waiting", "Z running"]])
        finally:
            server.shutdown()
            thread.join()
            server.server_close()

if __name__ == "__main__":
    unittest.main()
