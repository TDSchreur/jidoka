#!/usr/bin/env python3
"""Exercise attention and blocked status in Chromium."""
import unittest

from dashboard_browser import Handler, run_dashboard

AT = 1794081806000


def run(run_id, title, updated_at):
    return {
        "id": run_id, "title": title, "project": "/work/shop", "stage": "build",
        "status": "running", "needs_you": False, "updated_at": updated_at,
        "timeline": [{"at": AT, "stage": "build"}],
    }


COMPACTING = run("compacting", "Compacting run", AT + 2000)
WAITING = run("waiting", "Waiting for an answer", AT + 1000)


class AttentionHandler(Handler):
    def do_POST(self):
        if self.path in ("/block", "/resume"):
            compacting, waiting = self.runs
            if self.path == "/block":
                self.__class__.runs = [
                    {
                        **current, "status": "blocked", "needs_you": needs_you, "blocking_reason": reason,
                        "events": [{"at": current["updated_at"], "kind": "blocked", "stage": "build", "reason": reason}],
                    }
                    for current, reason, needs_you in (
                        (compacting, "Context compaction in progress", False),
                        (waiting, "Agent stopped; it may need you", True),
                    )
                ]
            else:
                self.__class__.runs = [compacting, {**waiting, "status": "running", "needs_you": False, "blocking_reason": None}]
            self.send_response(204)
            self.end_headers()
            return
        super().do_POST()


CHECK = """
<script>
const alerts = [];
window.Notification = class {
  static permission = 'granted';
  constructor(title) { alerts.push(title); }
};
localStorage.setItem('notifications', 'on');
const wait = (condition, what) => new Promise((resolve, reject) => {
  const started = Date.now();
  const timer = setInterval(() => {
    const value = condition();
    if (value) { clearInterval(timer); resolve(value); }
    else if (Date.now() - started > PAGE_WAIT_MS) {
      clearInterval(timer);
      reject(new Error('timed out waiting for ' + what));
    }
  }, 20);
});
const post = (path, data) => fetch(path, {method: 'POST', body: JSON.stringify(data)});
const row = (id) => document.querySelector('.line-open[data-run-key$="' + id + '"]');
const titleText = (selector) => [...document.querySelectorAll(selector)].map((el) => el.textContent.trim());
(async () => {
  const out = {};
  await wait(() => row('compacting') && row('waiting'), 'initial runs');
  await post('/block');
  await wait(() => document.title === '(1) Jidoka', 'one run needing attention');
  await wait(() => alerts.length, 'browser notification');
  out.alerts = [...alerts];
  out.names = titleText('.andon-run-title');
  out.order = titleText('.line-open');
  out.positions = titleText('.line-row .where');
  out.reasons = titleText('.line-row .line-reason');
  row('compacting').click();
  const detail = await wait(() => document.querySelector('.detail[open] .status-row.blocked'), 'compaction detail');
  out.detailReason = detail.querySelector('.status-reason').textContent.trim();
  const event = await wait(() => document.querySelector('.detail .pass-event.blocked'), 'block history');
  out.event = event.textContent.trim();
  document.querySelector('.detail-close').click();
  await wait(() => !document.querySelector('.detail[open]'), 'detail closed');
  document.querySelector('.andon-filter').click();
  out.filtered = await wait(() => !row('compacting') && titleText('.line-open'), 'waiting filter');
  await post('/resume');
  await wait(() => document.title === 'Jidoka' && !row('waiting'), 'attention cleared');
  out.clear = document.querySelector('.andon-summary [role="status"]').textContent.trim();
  document.querySelector('.andon-filter').click();
  await wait(() => row('compacting') && row('waiting'), 'filter cleared');
  out.afterResume = titleText('.line-row .where');
  await post('/result', out);
})().catch((error) => post('/result', {error: String(error)}));
</script>
"""


class AttentionTest(unittest.TestCase):
    def test_compaction_remains_blocked_without_attention(self):
        result = run_dashboard([dict(COMPACTING), dict(WAITING)], CHECK, handler=AttentionHandler)
        self.assertNotIn("error", result)
        self.assertEqual(len(result["alerts"]), 1)
        self.assertTrue(result["alerts"][0].endswith(": Waiting for an answer"))
        self.assertEqual(result["names"], ["Waiting for an answer"])
        self.assertEqual(result["order"], ["Waiting for an answer", "Compacting run"])
        self.assertEqual(result["positions"], ["Build, blocked · Needs you", "Build, blocked"])
        self.assertEqual(result["reasons"], ["Agent stopped; it may need you", "Context compaction in progress"])
        self.assertEqual(result["detailReason"], "Context compaction in progress")
        self.assertTrue(result["event"].startswith("Blocked: Context compaction in progress"))
        self.assertEqual(result["filtered"], ["Waiting for an answer"])
        self.assertEqual(result["clear"], "Nothing needs you.")
        self.assertEqual(result["afterResume"], ["Build, blocked", "Build"])


if __name__ == "__main__":
    unittest.main()
