#!/usr/bin/env python3
"""Exercise project hiding and restoration in Chromium."""
import http.server
import importlib.util
import os
import signal
import pathlib
import subprocess
import tempfile
import threading
import unittest

path = pathlib.Path(__file__).with_name("dashboard-project-rows.py")
spec = importlib.util.spec_from_file_location("project_rows", path)
rows = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rows)

rows.CHECK = """
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
setTimeout(() => fetch('/result', {method: 'POST', body: JSON.stringify({diagnostic: sessionStorage.getItem('hidden-test'), focus: document.activeElement.tagName, target: document.querySelectorAll('.project-toggle')[1]?.outerHTML, rows: document.querySelectorAll('.project-row').length})}), 24000);
const check = setInterval(() => {
  const projects = [...document.querySelectorAll('.project-row')];
  if (!sessionStorage.getItem('hidden-test')) {
    if (projects.length !== 2) return;
    projects[1].querySelector('.hide-project').click();
    sessionStorage.setItem('hidden-test', 'hiding');
  } else if (sessionStorage.getItem('hidden-test') === 'hiding') {
    if (document.activeElement !== document.querySelector('.hidden-projects summary')) return;
    sessionStorage.setItem('hidden-test', 'reload');
    location.reload();
  } else if (sessionStorage.getItem('hidden-test') === 'reload') {
    if (projects.length !== 1 || !document.querySelector('.hidden-projects button')) return;
    sessionStorage.setItem('hidden-waiting', String(document.querySelector('.toolbar').textContent.includes('Needs you (0)')));
    sessionStorage.setItem('hidden-test', 'watch-hidden');
  } else if (sessionStorage.getItem('hidden-test') === 'watch-hidden') {
    if (Number(sessionStorage.getItem('watch-hidden-polls') || 0) < 2) return;
    sessionStorage.setItem('hidden-test', 'restoring');
    document.querySelector('.hidden-projects').open = true;
    document.querySelector('.hidden-projects button').focus();
    document.querySelector('.hidden-projects button').click();
  } else if (projects.length === 2 && document.activeElement === projects[1].querySelector('.project-toggle') && Number(sessionStorage.getItem('restoring-polls') || 0) >= 2) {
    clearInterval(check);
    fetch('/result', { method: 'POST', body: JSON.stringify({
      names: projects.map(row => row.querySelector('.project-heading').textContent.trim()),
      stored: JSON.parse(localStorage.getItem('hidden-projects')),
      hiddenWaiting: sessionStorage.getItem('hidden-waiting'),
      notifications: Number(sessionStorage.getItem('notification-count') || 0),
      waiting: document.querySelector('.toolbar').textContent.includes('Needs you (1)')
    }) });
  }
}, 10);
</script>
"""


class HiddenProjectsTest(unittest.TestCase):
    def test_hide_persists_and_restore_recovers_project(self):
        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), rows.Handler)
        thread = threading.Thread(target=server.serve_forever)
        thread.start()
        try:
            with tempfile.TemporaryDirectory() as profile:
                process = subprocess.Popen(
                    ["chromium", "--headless", "--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage",
                     "--user-data-dir=" + profile, f"http://127.0.0.1:{server.server_port}/"],
                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True,
                )
                try:
                    result = rows.RESULTS.get(timeout=30)
                finally:
                    os.killpg(process.pid, signal.SIGTERM)
                    process.wait(timeout=10)
            self.assertNotIn("diagnostic", result, result)
            self.assertEqual(result["names"], ["▾alpha", "▾zeta"])
            self.assertEqual(result["stored"], [])
            self.assertTrue(result["waiting"])
            self.assertEqual(result["hiddenWaiting"], "true")
            self.assertEqual(result["notifications"], 0)
        finally:
            server.shutdown()
            thread.join()
            server.server_close()


if __name__ == "__main__":
    unittest.main()
