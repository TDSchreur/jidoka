#!/usr/bin/env python3
"""Exercise project deletion in Chromium."""
import http.server
import importlib.util
import json
import pathlib
import subprocess
import tempfile
import threading
import unittest

spec = importlib.util.spec_from_file_location('rows', pathlib.Path(__file__).with_name('dashboard-project-rows.py'))
rows = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rows)

rows.CHECK = '''<script>
let phase = 0;
const check = setInterval(() => {
  const projects = [...document.querySelectorAll('.project-row')];
  const dialog = document.querySelector('dialog.confirm');
  if (phase === 0 && projects.length === 2) {
    projects[1].querySelector('.delete-project').click(); phase = 1;
  } else if (phase === 1 && dialog?.open) {
    if (!dialog.textContent.includes('2 run records') || !dialog.textContent.includes('repository')) return;
    dialog.querySelector('.danger').click(); phase = 2;
  } else if (phase === 2 && projects.length === 1) {
    clearInterval(check);
    fetch('/result', {method: 'POST', body: JSON.stringify({name: projects[0].textContent, remaining: projects.length})});
  }
}, 10);
</script>'''

class Handler(rows.Handler):
    def do_DELETE(self):
        run_id = self.path.rsplit('/', 1)[-1]
        rows.RUNS[:] = [run for run in rows.RUNS if run['id'] != run_id]
        self.send_response(204)
        self.end_headers()

class DeleteProjectTest(unittest.TestCase):
    def test_deletes_all_selected_project_runs(self):
        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        thread = threading.Thread(target=server.serve_forever)
        thread.start()
        try:
            with tempfile.TemporaryDirectory() as profile:
                process = subprocess.Popen(['chromium', '--headless', '--no-sandbox', '--disable-gpu', '--disable-dev-shm-usage', '--user-data-dir=' + profile, f'http://127.0.0.1:{server.server_port}/'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                try:
                    result = rows.RESULTS.get(timeout=30)
                finally:
                    process.terminate()
                    process.wait(timeout=10)
            self.assertIn('alpha', result['name'])
            self.assertEqual({run['id'] for run in rows.RUNS}, {'2', '4'})
        finally:
            server.shutdown()
            thread.join()
            server.server_close()

if __name__ == '__main__':
    unittest.main()
