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
  const build = [...detail.querySelectorAll('.route > li')].find((li) => li.querySelector('h3').firstChild.textContent === 'Build');
  const updated = detail.querySelector('.status-updated');
  fetch('/result', {method:'POST', body:JSON.stringify({text:detail.textContent, skills:[...build.querySelectorAll('.skills li')].map((li) => li.textContent.trim()), updatedAt:updated?.querySelector('time')?.getAttribute('datetime'), updated:updated?.textContent.replace(/\s+/g, ' ').trim()})});
}, 10);
</script>
"""

class RunReportsTest(unittest.TestCase):
    def test_reports_show_without_tickets(self):
        result = run_dashboard(RUNS, CHECK, env={**os.environ, "TZ": "Europe/Amsterdam"}, extra_args=["--lang=en-US"])
        self.assertEqual(result["skills"], ["factory-fast"])
        self.assertIn("may be incomplete", result["text"])
        self.assertEqual(result["updatedAt"], "2026-11-07T20:03:26.000Z")
        self.assertRegex(result["updated"], r"^Updated (Yesterday|Sat Nov 7|Sat 7 Nov) 21:03$")

if __name__ == "__main__":
    unittest.main()
