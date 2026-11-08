#!/usr/bin/env python3
"""Exercise run reports in the rendered dashboard (requires Chromium)."""
import os
import unittest

from dashboard_browser import run_dashboard

RUNS = [{"id": "1", "title": "Run", "project": "/work/project", "stage": "build", "status": "running", "updated_at": 1794081806000,
              "tickets": [], "run_reports": [{"kind": "skill", "text": "factory-fast", "stage": "build", "at": 1700000000000}]}]
CHECK = """
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
        result = run_dashboard(RUNS, CHECK, env={**os.environ, "TZ": "Europe/Amsterdam"}, extra_args=["--lang=en-US"])
        self.assertIn("Skill: factory-fast", result["text"])
        self.assertIn("may be incomplete", result["text"])
        self.assertEqual(result["time"], "2023-11-14T22:13:20.000Z")
        self.assertIn("07/11/2026, 21:03:26", result["updated"])

if __name__ == "__main__":
    unittest.main()
