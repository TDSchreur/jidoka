#!/usr/bin/env python3
"""Exercise project deletion in Chromium."""
import unittest

from dashboard_browser import Handler, run_dashboard

RUNS = [
    {'id': '1', 'title': 'Z running', 'project': '/work/zeta', 'stage': 'build', 'status': 'running', 'updated_at': 200},
    {'id': '2', 'title': 'A running', 'project': '/work/alpha', 'stage': 'build', 'status': 'running', 'updated_at': 300},
    {'id': '3', 'title': 'Z waiting', 'project': '/work/zeta', 'stage': 'build', 'status': 'blocked', 'updated_at': 100},
    {'id': '4', 'title': 'A define', 'project': '/work/alpha', 'stage': 'define', 'status': 'running', 'updated_at': 400},
]
CHECK = '''<script>
let phase = 0;
const check = setInterval(() => {
  const menu = document.querySelector('.projects-menu');
  const dialog = document.querySelector('dialog.confirm');
  const names = [...document.querySelectorAll('.projects-list .projects-name')].map(el => el.textContent.trim());
  if (phase === 0 && names.length === 2) {
    menu.open = true;
    menu.querySelector('[aria-label="Delete zeta"]').click(); phase = 1;
  } else if (phase === 1 && dialog?.open) {
    if (!dialog.textContent.includes('Delete zeta?') || !dialog.textContent.includes('2 run records') || !dialog.textContent.includes('repository')) return;
    dialog.querySelector('.danger').click(); phase = 2;
  } else if (phase === 2 && names.length === 1 && document.querySelectorAll('.line-row').length === 2) {
    clearInterval(check);
    fetch('/result', {method: 'POST', body: JSON.stringify({names, remaining: names.length})});
  }
}, 10);
</script>'''

class DeleteHandler(Handler):
    def do_DELETE(self):
        run_id = self.path.rsplit('/', 1)[-1]
        self.runs[:] = [run for run in self.runs if run['id'] != run_id]
        self.send_response(204)
        self.end_headers()

class DeleteProjectTest(unittest.TestCase):
    def test_deletes_all_selected_project_runs(self):
        runs = [dict(run) for run in RUNS]
        result = run_dashboard(runs, CHECK, handler=DeleteHandler)
        self.assertEqual(result['names'], ['alpha'])
        self.assertEqual({run['id'] for run in runs}, {'2', '4'})

if __name__ == '__main__':
    unittest.main()
