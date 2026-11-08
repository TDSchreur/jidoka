#!/usr/bin/env python3
"""Browser behavior test for the line and the Projects menu; run with python3 tests/dashboard-project-rows.py (requires Chromium or Chrome)."""
import unittest

from dashboard_browser import run_dashboard

RUNS = [
    {"id": "1", "title": "Z running", "project": "/work/zeta", "stage": "build", "status": "running", "updated_at": 350},
    {"id": "2", "title": "A running", "project": "/work/alpha", "stage": "build", "status": "running", "updated_at": 300},
    {"id": "3", "title": "Z waiting", "project": "/work/zeta", "stage": "build", "status": "blocked", "updated_at": 100},
    {"id": "4", "title": "A define", "project": "/work/alpha", "stage": "define", "status": "running", "updated_at": 400},
]
CHECK = """
<script>
const check = setInterval(() => {
  const rows = [...document.querySelectorAll('.line-row')];
  if (rows.length !== 4) return;
  clearInterval(check);
  const text = (el) => el.textContent.trim();
  fetch("/result", {method: "POST", body: JSON.stringify({
    titles: rows.map(row => text(row.querySelector('.line-open'))),
    projects: rows.map(row => text(row.querySelector('.meta-project'))),
    where: rows.map(row => text(row.querySelector('.where'))),
    menu: text(document.querySelector('.projects-menu summary')),
    menuRows: [...document.querySelectorAll('.projects-list li')].map(li => [text(li.querySelector('.projects-name')), text(li.querySelector('.projects-count'))]),
    andon: text(document.querySelector('.andon-summary')),
  })});
}, 10);
</script>
"""

class ProjectRowsTest(unittest.TestCase):
    def test_the_line_puts_blocked_runs_first_and_the_menu_lists_each_project(self):
        result = run_dashboard(RUNS, CHECK)
        # One order across both projects: blocked first, then the most recently updated. Sorting within each
        # project would give alpha's runs before zeta's.
        self.assertEqual(result["titles"], ["Z waiting", "A define", "Z running", "A running"])
        self.assertEqual(result["projects"], ["zeta", "alpha", "zeta", "alpha"])
        self.assertEqual(result["where"], ["Build, blocked", "Define", "Build", "Build"])
        self.assertEqual(result["menu"], "Projects (2)")
        self.assertEqual(result["menuRows"], [["alpha", "2 runs"], ["zeta", "2 runs"]])
        self.assertEqual(result["andon"], "1 run needs you")

if __name__ == "__main__":
    unittest.main()
