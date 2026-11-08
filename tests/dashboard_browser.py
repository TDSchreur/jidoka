"""Shared harness for the tests/dashboard-*.py browser tests (set CHROMIUM to pick the browser).

`run_dashboard` serves the real dashboard files from 127.0.0.1 on a free port with the runs and the check script a
test passes, opens them in a headless browser with a throwaway profile, and returns what the page POSTed to /result.

Every page gets the global `PAGE_WAIT_MS`, the one in-page wait budget for the tests' own waits. It is generous for a
cold browser start and well inside the harness wait (`WAIT_SECONDS`). When the browser exits before the page posts a
result, `run_dashboard` retries once with a fresh profile; a browser that keeps running without a result fails at the
wait limit.

Written for Linux and macOS, run on Linux in CI, and untested on Windows (set CHROMIUM elsewhere).
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
import time
import urllib.parse

ROOT = pathlib.Path(__file__).resolve().parents[1] / "dashboard"
WAIT_SECONDS = 120
PAGE_WAIT_MS = 60000
RETRIES = 1
POLL_SECONDS = 0.5


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
        route = urllib.parse.unquote(urllib.parse.urlsplit(self.path).path)
        if route == "/api/runs":
            data, kind = json.dumps(self.runs).encode(), "application/json"
        elif route == "/":
            data, kind = (ROOT / "index.html").read_bytes() + self.check.encode(), "text/html"
        elif route.startswith("/assets/"):
            try:
                path = (ROOT / route.removeprefix("/assets/")).resolve()
                found = path.is_relative_to(ROOT.resolve()) and path.is_file()
            except (OSError, ValueError):
                found = False
            if not found:
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
    group = os.name != "nt"
    if group:
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
    else:
        try:
            subprocess.run(["taskkill", "/T", "/F", "/PID", str(process.pid)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        except FileNotFoundError:
            process.kill()
    try:
        process.wait(timeout=10)
    except subprocess.TimeoutExpired:
        if group:
            os.killpg(process.pid, signal.SIGKILL)
        else:
            process.kill()
        process.wait()


def run_dashboard(runs, check, size=(1100, 700), handler=Handler, env=None, extra_args=()):
    """Opens the dashboard with `runs` and the `check` script in a headless browser; returns the page's /result."""
    results = queue.Queue()
    check = f"<script>window.PAGE_WAIT_MS = {PAGE_WAIT_MS};</script>" + check
    handler = type("TestHandler", (handler,), {"runs": runs, "check": check, "results": results})
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    thread = threading.Thread(target=server.serve_forever)
    thread.start()
    try:
        for attempt in range(RETRIES + 1):
            outcome = attempt_once(server, results, size, env, extra_args, last=attempt == RETRIES)
            if outcome is not None:
                return outcome
    finally:
        server.shutdown()
        thread.join()
        server.server_close()


def attempt_once(server, results, size, env, extra_args, last):
    """One browser run with a fresh profile: the page's result, or None when the browser exited without one and `last` is false."""
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
            deadline = time.monotonic() + WAIT_SECONDS
            while time.monotonic() < deadline:
                try:
                    return results.get(timeout=POLL_SECONDS)
                except queue.Empty:
                    pass
                if process.poll() is not None:
                    # A result posted just before the exit still counts.
                    try:
                        return results.get(timeout=1)
                    except queue.Empty:
                        break
            if process.poll() is not None and not last:
                return None
            log.seek(0)
            tail = "".join(log.readlines()[-40:])
            raise RuntimeError(f"No result from the browser within {WAIT_SECONDS}s (exit code {process.poll()}). Browser stderr:\n{tail}")
        finally:
            stop(process)
