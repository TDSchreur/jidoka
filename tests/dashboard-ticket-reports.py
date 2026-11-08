#!/usr/bin/env python3
"""Exercise ticket reports in the rendered dashboard (requires Chromium)."""
import unittest

from dashboard_browser import run_dashboard

RUNS = [{"id": "1", "title": "Run", "project": "/work/project", "stage": "build", "status": "running", "updated_at": 100,
              "tickets": [{"reference": "one", "title": "First", "status": "done"}, {"reference": "two", "title": "Second", "status": "pending"}],
              "ticket_reports": [{"ticket": "one", "kind": "skill", "text": "tdd", "stage": "build", "at": 1700000000000},
                                 {"ticket": "one", "kind": "activity", "text": "Covered the first case", "stage": "build", "at": 1700000000000}]}]
CHECK = """
<script>
const check = setInterval(() => {
  const button = document.querySelector('.line-open');
  if (!button) return;
  button.click();
  const items = [...document.querySelectorAll('.ticket-list > li')];
  if (items.length !== 2) return;
  clearInterval(check);
  fetch('/result', {method:'POST', body:JSON.stringify({items:items.map(item => item.textContent), skills:[...items[0].querySelectorAll('.skills li')].map(li => li.textContent.trim()), time:items[0].querySelector('time')?.getAttribute('datetime'), insideBuild:items.every(item => item.closest('.route > li').querySelector('h3').firstChild.textContent === 'Build')})});
}, 10);
</script>
"""

class TicketReportsTest(unittest.TestCase):
    def test_reports_belong_to_their_ticket(self):
        result = run_dashboard(RUNS, CHECK)
        self.assertEqual(result["skills"], ["tdd"])
        self.assertIn("Covered the first case", result["items"][0])
        self.assertNotIn("tdd", result["items"][1])
        self.assertNotIn("Covered", result["items"][1])
        self.assertTrue(result["insideBuild"])
        self.assertEqual(result["time"], "2023-11-14T22:13:20.000Z")

if __name__ == "__main__":
    unittest.main()
