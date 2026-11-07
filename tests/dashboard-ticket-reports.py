#!/usr/bin/env python3
"""Exercise ticket reports in the rendered dashboard (requires Chromium)."""
import http.server
import importlib.util
import pathlib
import subprocess
import tempfile
import threading
import unittest

spec = importlib.util.spec_from_file_location("rows", pathlib.Path(__file__).with_name("dashboard-project-rows.py"))
rows = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rows)
rows.RUNS = [{"id": "1", "title": "Run", "project": "/work/project", "stage": "build", "status": "running", "updated_at": 100,
              "tickets": [{"reference": "one", "title": "First", "status": "done"}, {"reference": "two", "title": "Second", "status": "pending"}],
              "ticket_reports": [{"ticket": "one", "kind": "skill", "text": "tdd", "stage": "build", "at": 1700000000000}]}]
rows.CHECK = """
<script>
const check = setInterval(() => {
  const button = document.querySelector('.line-open');
  if (!button) return;
  button.click();
  const items = [...document.querySelectorAll('.ticket-list > li')];
  if (items.length !== 2) return;
  clearInterval(check);
  fetch('/result', {method:'POST', body:JSON.stringify({items:items.map(item => item.textContent), time:items[0].querySelector('time')?.getAttribute('datetime')})});
}, 10);
</script>
"""

class TicketReportsTest(unittest.TestCase):
    def test_reports_belong_to_their_ticket(self):
        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), rows.Handler)
        thread = threading.Thread(target=server.serve_forever)
        thread.start()
        try:
            with tempfile.TemporaryDirectory() as profile:
                process = subprocess.Popen(["chromium", "--headless", "--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage", "--user-data-dir=" + profile, f"http://127.0.0.1:{server.server_port}/"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                try:
                    result = rows.RESULTS.get(timeout=30)
                finally:
                    process.terminate()
                    process.wait(timeout=10)
            self.assertIn("Skill: tdd", result["items"][0])
            self.assertIn("build", result["items"][0])
            self.assertIn("may be incomplete", result["items"][0])
            self.assertNotIn("tdd", result["items"][1])
            self.assertIn("No reports recorded", result["items"][1])
            self.assertEqual(result["time"], "2023-11-14T22:13:20.000Z")
        finally:
            server.shutdown()
            thread.join()
            server.server_close()

if __name__ == "__main__":
    unittest.main()
