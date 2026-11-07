#!/usr/bin/env python3
"""Exercise run reports in the rendered dashboard (requires Chromium)."""
import http.server
import importlib.util
import os
import pathlib
import subprocess
import tempfile
import threading
import unittest

spec = importlib.util.spec_from_file_location("rows", pathlib.Path(__file__).with_name("dashboard-project-rows.py"))
rows = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rows)
rows.RUNS = [{"id": "1", "title": "Run", "project": "/work/project", "stage": "build", "status": "running", "updated_at": 1794081806000,
              "tickets": [], "run_reports": [{"kind": "skill", "text": "factory-fast", "stage": "build", "at": 1700000000000}]}]
rows.CHECK = """
<script>
const check = setInterval(() => {
  const button = document.querySelector('.line-open');
  if (!button) return;
  button.click();
  const detail = document.querySelector('.detail-body');
  if (!detail || !detail.textContent.includes('factory-fast')) return;
  clearInterval(check);
  const updatedLabel = [...detail.querySelectorAll('dt')].find((label) => label.textContent.trim() === 'Updated');
  fetch('/result', {method:'POST', body:JSON.stringify({text:detail.textContent, time:detail.querySelector('time[datetime="2023-11-14T22:13:20.000Z"]')?.getAttribute('datetime'), updated:updatedLabel?.nextElementSibling?.textContent.trim()})});
}, 10);
</script>
"""

class RunReportsTest(unittest.TestCase):
    def test_reports_show_without_tickets(self):
        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), rows.Handler)
        thread = threading.Thread(target=server.serve_forever)
        thread.start()
        try:
            with tempfile.TemporaryDirectory() as profile:
                process = subprocess.Popen(
                    [
                        "chromium", "--headless", "--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage",
                        "--lang=en-US", "--user-data-dir=" + profile, f"http://127.0.0.1:{server.server_port}/",
                    ],
                    env={**os.environ, "TZ": "Europe/Amsterdam"},
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                )
                try:
                    result = rows.RESULTS.get(timeout=30)
                finally:
                    process.terminate()
                    try:
                        process.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()
            self.assertIn("Skill: factory-fast", result["text"])
            self.assertIn("may be incomplete", result["text"])
            self.assertEqual(result["time"], "2023-11-14T22:13:20.000Z")
            self.assertIn("07/11/2026, 21:03:26", result["updated"])
        finally:
            server.shutdown()
            thread.join()
            server.server_close()

if __name__ == "__main__":
    unittest.main()
