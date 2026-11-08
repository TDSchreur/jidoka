#!/usr/bin/env python3
"""Exercise project hiding and restoration in Chromium."""
import unittest

from dashboard_browser import run_dashboard

RUNS = [
    {"id": "1", "title": "Z running", "project": "/work/zeta", "stage": "build", "status": "running", "updated_at": 200},
    {"id": "2", "title": "A running", "project": "/work/alpha", "stage": "build", "status": "running", "updated_at": 300},
    {"id": "3", "title": "Z waiting", "project": "/work/zeta", "stage": "build", "status": "blocked", "updated_at": 100},
    {"id": "4", "title": "A define", "project": "/work/alpha", "stage": "define", "status": "running", "updated_at": 400},
]
CHECK = """
<script>
localStorage.setItem('notifications', 'on');
const originalFetch = window.fetch;
window.fetch = async (...args) => {
  const response = await originalFetch(...args);
  if (args[0] !== '/api/runs') return response;
  const phase = sessionStorage.getItem('hidden-test');
  const data = await response.json();
  if (phase === 'reload') data.find(run => run.id === '3').status = 'running';
  if (phase === 'watch-hidden' || phase === 'restoring') {
    sessionStorage.setItem(phase + '-polls', String(Number(sessionStorage.getItem(phase + '-polls') || 0) + 1));
  }
  return new Response(JSON.stringify(data), {status: response.status, headers: response.headers});
};
window.Notification = class {
  static permission = 'granted';
  constructor() { sessionStorage.setItem('notification-count', String(Number(sessionStorage.getItem('notification-count') || 0) + 1)); }
};
const text = (el) => el.textContent.trim();
const menu = () => document.querySelector('.projects-menu');
const menuButton = (label) => menu().querySelector(`[aria-label="${label}"]`);
setTimeout(() => fetch('/result', {method: 'POST', body: JSON.stringify({diagnostic: sessionStorage.getItem('hidden-test'), focus: document.activeElement.tagName, target: menu()?.outerHTML, rows: document.querySelectorAll('.line-row').length})}), 24000);
const check = setInterval(() => {
  const rows = [...document.querySelectorAll('.line-row')];
  const phase = sessionStorage.getItem('hidden-test');
  if (!phase) {
    if (rows.length !== 4) return;
    menu().open = true;
    // A click gives the button focus, as it would for a person.
    menuButton('Hide zeta').focus();
    menuButton('Hide zeta').click();
    sessionStorage.setItem('hidden-test', 'hiding');
  } else if (phase === 'hiding') {
    // The zeta runs leave the line, and focus stays on the button, which now restores.
    if (rows.length !== 2 || document.activeElement !== menuButton('Restore zeta')) return;
    sessionStorage.setItem('hidden-test', 'reload');
    location.reload();
  } else if (phase === 'reload') {
    if (rows.length !== 2 || !menuButton('Restore zeta')) return;
    sessionStorage.setItem('hidden-stored', localStorage.getItem('hidden-projects'));
    sessionStorage.setItem('hidden-waiting', String(text(document.querySelector('.andon-summary')).includes('Nothing needs you.')));
    sessionStorage.setItem('hidden-test', 'watch-hidden');
  } else if (phase === 'watch-hidden') {
    if (Number(sessionStorage.getItem('watch-hidden-polls') || 0) < 2) return;
    sessionStorage.setItem('hidden-test', 'restoring');
    menu().open = true;
    menuButton('Restore zeta').focus();
    menuButton('Restore zeta').click();
  } else if (rows.length === 4 && document.activeElement === menuButton('Hide zeta') && Number(sessionStorage.getItem('restoring-polls') || 0) >= 2) {
    clearInterval(check);
    fetch('/result', { method: 'POST', body: JSON.stringify({
      names: [...document.querySelectorAll('.projects-list .projects-name')].map(text),
      storedWhileHidden: JSON.parse(sessionStorage.getItem('hidden-stored')),
      stored: JSON.parse(localStorage.getItem('hidden-projects')),
      hiddenWaiting: sessionStorage.getItem('hidden-waiting'),
      notifications: Number(sessionStorage.getItem('notification-count') || 0),
      waiting: text(document.querySelector('.andon-summary')).includes('1 run needs you')
    }) });
  }
}, 10);
</script>
"""


class HiddenProjectsTest(unittest.TestCase):
    def test_hide_persists_and_restore_recovers_project(self):
        result = run_dashboard(RUNS, CHECK)
        self.assertNotIn("diagnostic", result, result)
        self.assertEqual(result["names"], ["alpha", "zeta"])
        self.assertEqual(result["storedWhileHidden"], ["/work/zeta"])
        self.assertEqual(result["stored"], [])
        self.assertTrue(result["waiting"])
        self.assertEqual(result["hiddenWaiting"], "true")
        self.assertEqual(result["notifications"], 0)


if __name__ == "__main__":
    unittest.main()
