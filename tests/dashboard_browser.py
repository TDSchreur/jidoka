"""Shared harness for the tests/dashboard-*.py browser tests (requires Chromium or Chrome; set CHROMIUM to pick one).

Each test passes the runs the dashboard should list and a `<script>` that drives the page and POSTs its findings
to /result. Written for Linux and macOS; on Windows it should work with CHROMIUM set to chrome.exe, but CI and
scripts/verify.sh run it on Linux only and it has not been tried on Windows. `run_dashboard` serves the real dashboard files from 127.0.0.1 on a free port, opens them in a headless
browser with a throwaway profile, and returns what the page reported.
"""
import http.server
import json
import os
import pathlib
import queue
import shutil
import signal
import subprocess
import tempfile
import threading

ROOT = pathlib.Path(__file__).resolve().parents[1] / "dashboard"
WAIT_SECONDS = 120


def find_browser():
    """The CHROMIUM environment variable, else the first of chromium, chromium-browser and google-chrome on PATH.

    Those names are the Linux ones; on other platforms set CHROMIUM to the browser's path.
    """
    found = os.environ.get("CHROMIUM") or next(filter(None, map(shutil.which, ["chromium", "chromium-browser", "google-chrome"])), None)
    if not found:
        raise RuntimeError("No Chromium found. Install chromium or google-chrome, or set CHROMIUM to its path.")
    return found


class Handler(http.server.BaseHTTPRequestHandler):
    """Serves the dashboard with `check` appended to the page, `runs` as the API and collects the page's result.

    `run_dashboard` fills in `runs`, `check` and `results` on a subclass; tests subclass it to add endpoints.
    """
    runs = []
    check = ""
    results = None

    def do_POST(self):
        if self.path != "/result":
            self.send_error(404)
            return
        data = self.rfile.read(int(self.headers["Content-Length"]))
        self.results.put(json.loads(data))
        self.send_response(204)
        self.end_headers()

    def do_GET(self):
        if self.path == "/api/runs":
            data, kind = json.dumps(self.runs).encode(), "application/json"
        elif self.path == "/":
            data, kind = (ROOT / "index.html").read_bytes() + self.check.encode(), "text/html"
        elif self.path.startswith("/assets/"):
            path = (ROOT / self.path.removeprefix("/assets/")).resolve()
            if not path.is_relative_to(ROOT.resolve()) or not path.is_file():
                self.send_error(404)
                return
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


def stop(process):
    """Stops the browser and its helper processes, so none still writes into the profile that is about to be removed."""
    if os.name == "nt":
        subprocess.run(["taskkill", "/T", "/F", "/PID", str(process.pid)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    else:
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
    try:
        process.wait(timeout=10)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()


def run_dashboard(runs, check, size=(1100, 700), handler=Handler, env=None, extra_args=()):
    """Opens the dashboard with `runs` and the `check` script in a headless browser; returns the page's /result."""
    results = queue.Queue()
    handler = type("TestHandler", (handler,), {"runs": runs, "check": check, "results": results})
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    thread = threading.Thread(target=server.serve_forever)
    thread.start()
    try:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as profile, \
                tempfile.TemporaryFile(mode="w+", errors="replace") as log:
            # On POSIX the browser gets its own session, so `stop` can end its helper processes with it.
            process = subprocess.Popen(
                [find_browser(), "--headless", "--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage",
                 "--no-first-run", "--no-default-browser-check", "--disable-extensions",
                 f"--window-size={size[0]},{size[1]}", *extra_args, "--user-data-dir=" + profile,
                 f"http://127.0.0.1:{server.server_port}/"],
                stdout=subprocess.DEVNULL, stderr=log, env=env, start_new_session=os.name != "nt",
            )
            try:
                try:
                    return results.get(timeout=WAIT_SECONDS)
                except queue.Empty:
                    log.seek(0)
                    tail = "".join(log.readlines()[-40:])
                    raise RuntimeError(f"No result from the browser within {WAIT_SECONDS}s (exit code {process.poll()}). Browser stderr:\n{tail}") from None
            finally:
                stop(process)
    finally:
        server.shutdown()
        thread.join()
        server.server_close()
