#!/usr/bin/env python3
"""Browser behavior test; run with python3 tests/dashboard-help.py (requires Chromium)."""
import http.server
import json
import pathlib
import queue
import subprocess
import tempfile
import threading
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1] / "dashboard"
RESULTS = queue.Queue()
RUNS = [{"id": "1", "title": "A run", "project": "/work/alpha", "stage": "build", "status": "running", "updated_at": 300}]
TABS = ["Overview", "Stages", "Workflows", "Skills", "Dashboard", "Commands", "Model routing"]
# Drives the Help like a person with a keyboard and reports what it saw.
CHECK = """
<script>
const wait = (ok) => new Promise((resolve) => { const t = setInterval(() => { const v = ok(); if (v) { clearInterval(t); resolve(v); } }, 20); });
const key = (el, k) => el.dispatchEvent(new KeyboardEvent('keydown', {key: k, bubbles: true, cancelable: true}));
const state = () => {
  const dialog = document.querySelector('dialog.help');
  const tabs = [...dialog.querySelectorAll('[role="tab"]')];
  const selected = tabs.filter((t) => t.getAttribute('aria-selected') === 'true');
  const panel = dialog.querySelector('[role="tabpanel"]');
  return {
    selected: selected.map((t) => t.textContent.trim()),
    tabbable: tabs.filter((t) => t.tabIndex === 0).map((t) => t.textContent.trim()),
    focused: document.activeElement?.textContent.trim(),
    panelLabel: panel && document.getElementById(panel.getAttribute('aria-labelledby'))?.textContent.trim(),
    controls: selected.length ? selected[0].getAttribute('aria-controls') === panel?.id : false,
    headings: panel ? [...panel.querySelectorAll('h3')].map((h) => h.textContent.trim()) : [],
    words: panel ? panel.textContent.trim().split(/\\s+/).length : 0,
    scrolls: panel ? panel.scrollHeight > panel.clientHeight : false,
    panelFocusable: panel ? panel.tabIndex === 0 : false,
    // Words that run into an inline element, such as "as<code>factory</code>": the text before it ends in a
    // letter or the text after it starts with one.
    glued: panel ? [...panel.querySelectorAll('code, strong, em')].flatMap((el) => {
      const before = el.previousSibling?.nodeType === 3 ? el.previousSibling.nodeValue : '';
      const after = el.nextSibling?.nodeType === 3 ? el.nextSibling.nodeValue : '';
      return [/[A-Za-z0-9]$/.test(before) ? 'before: ' + before.slice(-25) + '|' + el.textContent.slice(0, 20) : null,
              /^[A-Za-z0-9]/.test(after) ? 'after: ' + el.textContent.slice(-20) + '|' + after.slice(0, 25) : null].filter(Boolean);
    }) : [],
  };
};
(async () => {
  const opener = await wait(() => document.querySelector('.help-open'));
  opener.focus();
  opener.click();
  const dialog = await wait(() => document.querySelector('dialog.help[open]'));
  const tabs = [...dialog.querySelectorAll('[role="tab"]')];
  const out = {tabs: tabs.map((t) => t.textContent.trim()), listLabel: dialog.querySelector('[role="tablist"]')?.getAttribute('aria-label'),
               title: dialog.querySelector('h2')?.textContent.trim(), initial: state(), inDialog: dialog.contains(document.activeElement)};
  // Each tab by keyboard: Right moves on, End goes last, Left goes back, Home goes first.
  const first = tabs[0];
  first.focus();
  key(first, 'ArrowRight'); await wait(() => document.activeElement === tabs[1]); out.afterRight = state();
  key(tabs[1], 'End'); await wait(() => document.activeElement === tabs[tabs.length - 1]); out.afterEnd = state();
  key(tabs[tabs.length - 1], 'ArrowRight'); await wait(() => document.activeElement === tabs[0]); out.wrapped = state();
  key(tabs[0], 'ArrowLeft'); await wait(() => document.activeElement === tabs[tabs.length - 1]); out.wrappedBack = state();
  key(tabs[tabs.length - 1], 'Home'); await wait(() => document.activeElement === tabs[0]); out.afterHome = state();
  // Every tab by click: its content, and whether it needs scrolling in this window.
  out.every = {};
  for (const tab of tabs) { tab.click(); await wait(() => tab.getAttribute('aria-selected') === 'true'); out.every[tab.textContent.trim()] = state(); }
  // Closing returns focus to the button that opened the Help.
  dialog.querySelector('.help-close').click();
  await wait(() => !document.querySelector('dialog.help'));
  out.focusBack = document.activeElement === opener;
  await fetch("/result", {method: "POST", body: JSON.stringify(out)});
})().catch((e) => fetch("/result", {method: "POST", body: JSON.stringify({error: String(e)})}));
</script>
"""

class Handler(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        if self.path != "/result":
            self.send_error(404)
            return
        data = self.rfile.read(int(self.headers["Content-Length"]))
        RESULTS.put(json.loads(data))
        self.send_response(204)
        self.end_headers()

    def do_GET(self):
        if self.path == "/api/runs":
            data, kind = json.dumps(RUNS).encode(), "application/json"
        elif self.path == "/":
            data, kind = (ROOT / "index.html").read_bytes() + CHECK.encode(), "text/html"
        elif self.path.startswith("/assets/"):
            path = ROOT / self.path.removeprefix("/assets/")
            if not path.is_file():
                self.send_error(404)
                return
            data = path.read_bytes()
            kind = "text/javascript" if path.suffix == ".js" else "text/css"
        else:
            self.send_error(404)
            return
        self.send_response(200)
        self.send_header("Content-Type", kind)
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *args):
        pass

def run_browser(width, height):
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever)
    thread.start()
    try:
        with tempfile.TemporaryDirectory() as profile:
            process = subprocess.Popen(
                ["chromium", "--headless", "--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage",
                 f"--window-size={width},{height}", "--user-data-dir=" + profile, f"http://127.0.0.1:{server.server_port}/"],
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            )
            try:
                return RESULTS.get(timeout=60)
            finally:
                process.terminate()
                process.wait(timeout=10)
    finally:
        server.shutdown()
        thread.join()
        server.server_close()

class HelpTest(unittest.TestCase):
    def check(self, width, height):
        result = run_browser(width, height)
        self.assertNotIn("error", result, result)
        return result

    def test_tabs_work_with_a_keyboard_and_each_panel_holds_a_lot(self):
        result = self.check(1100, 700)
        self.assertEqual(result["title"], "Help")
        self.assertEqual(result["tabs"], TABS)
        self.assertTrue(result["listLabel"])
        self.assertTrue(result["inDialog"])
        # Only the selected tab is in the tab order; its panel is labelled by it and reachable for scrolling.
        initial = result["initial"]
        self.assertEqual(initial["selected"], ["Overview"])
        self.assertEqual(initial["tabbable"], ["Overview"])
        self.assertEqual(initial["panelLabel"], "Overview")
        self.assertTrue(initial["controls"] and initial["panelFocusable"])
        # Arrow keys, Home and End move focus and select, wrapping at the ends.
        for name, expected in [("afterRight", "Stages"), ("afterEnd", "Model routing"), ("wrapped", "Overview"),
                               ("wrappedBack", "Model routing"), ("afterHome", "Overview")]:
            state = result[name]
            self.assertEqual(state["selected"], [expected], name)
            self.assertEqual(state["tabbable"], [expected], name)
            self.assertEqual(state["focused"], expected, name)
            self.assertEqual(state["panelLabel"], expected, name)
        # Every tab has real content, with headings, and the long ones need the panel's own scrolling.
        for tab in TABS:
            state = result["every"][tab]
            self.assertEqual(state["panelLabel"], tab)
            self.assertGreaterEqual(len(state["headings"]), 2, tab)
            self.assertGreaterEqual(state["words"], 150, tab)
            self.assertEqual(state["glued"], [], tab)
        for tab in ["Stages", "Workflows", "Skills"]:
            self.assertTrue(result["every"][tab]["scrolls"], tab)
        self.assertTrue(result["focusBack"])

    def test_the_narrow_screen_keeps_every_tab_reachable(self):
        result = self.check(400, 700)
        self.assertEqual(result["tabs"], TABS)
        self.assertTrue(result["every"]["Skills"]["scrolls"])
        self.assertTrue(result["focusBack"])

if __name__ == "__main__":
    unittest.main()
