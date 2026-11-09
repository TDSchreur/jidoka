#!/usr/bin/env python3
"""Exercise the Reader (a run's local spec or ticket as a document) in Chromium; run with python3 tests/dashboard-reader.py."""
import json
import unittest
import urllib.parse

from dashboard_browser import Handler, run_dashboard

AT = 1794081806000
HTTP_SPEC = "https://github.com/acme/shop/issues/230"
LOCAL = {
    "id": "local", "title": "Local run", "project": "/work/shop", "stage": "build", "status": "running", "updated_at": AT,
    "spec": "docs/spec.md",
    "tickets": [{"reference": "docs/ticket-1.md", "title": "Local ticket", "status": "open"},
                {"reference": "https://github.com/acme/shop/issues/231", "title": "Web ticket", "status": "open"}],
}
GONE = {"id": "gone", "title": "Gone run", "project": "/work/shop", "stage": "build", "status": "running", "updated_at": AT - 2,
        "spec": "docs/old.md", "tickets": []}
WEB = {"id": "web", "title": "Web run", "project": "/work/shop", "stage": "build", "status": "running", "updated_at": AT - 1,
       "spec": HTTP_SPEC, "tickets": []}
DOCUMENTS = {
    "docs/spec.md": {"name": "spec.md", "html": "<h1>The spec</h1><p>Some <strong>text</strong>.</p>"},
    "docs/ticket-1.md": {"name": "ticket-1.md", "html": "<h2>Ticket one</h2><ul><li>item</li></ul>"},
}


class DocumentHandler(Handler):
    def do_GET(self):
        parts = urllib.parse.urlsplit(self.path)
        if parts.path in ("/api/runs/local/document", "/api/runs/gone/document"):
            reference = urllib.parse.parse_qs(parts.query).get("reference", [""])[0]
            found = DOCUMENTS.get(reference)
            body = json.dumps(found if found else {"available": False, "path": reference}).encode()
            self.send_response(200 if found else 404)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(body)
            return
        super().do_GET()


CHECK = """
<script>
const wait = (ok, what) => new Promise((resolve, reject) => {
  const started = Date.now();
  const t = setInterval(() => { const v = ok(); if (v) { clearInterval(t); resolve(v); } else if (Date.now() - started > PAGE_WAIT_MS) { clearInterval(t); reject(new Error('timed out waiting for ' + what)); } }, 20);
});
const text = (el) => (el ? el.textContent.replace(/\\s+/g, ' ').trim() : null);
const post = (data) => fetch('/result', {method: 'POST', body: JSON.stringify(data)});
const key = (k) => document.activeElement.dispatchEvent(new KeyboardEvent('keydown', {key: k, bubbles: true, cancelable: true}));
const rowOf = (id) => wait(() => [...document.querySelectorAll('[data-run-key]')].find((el) => el.dataset.runKey.endsWith('\\n' + id))?.closest('li'), 'row ' + id);
const views = (root) => [...root.querySelectorAll('button')].filter((b) => /^View/.test(text(b)));
(async () => {
  const out = {};
  const web = await rowOf('web');
  out.webRowLink = web.querySelector('.links a[href]')?.getAttribute('href');
  out.webRowViews = views(web).length;
  const row = await rowOf('local');
  out.rowViews = views(row).length;
  out.rowSpecLinks = [...row.querySelectorAll('.links a')].length;
  row.querySelector('[data-run-key]').click();
  const sheet = await wait(() => document.querySelector('dialog.detail[open]'), 'sheet');
  out.sheetViews = views(sheet.querySelector('.status-row')).length;
  const titleLinks = [...sheet.querySelectorAll('.ticket-name button')].map(text);
  out.titleLinks = titleLinks;
  out.webTicketLinks = [...sheet.querySelectorAll('.ticket-name a')].map(text);
  // Open the spec from the sheet.
  views(sheet.querySelector('.status-row'))[0].click();
  const reader = await wait(() => document.querySelector('dialog.reader[open]'), 'reader');
  await wait(() => reader.querySelector('.reader-doc h1'), 'document');
  out.specName = text(reader.querySelector('h2'));
  out.specBody = text(reader.querySelector('.reader-doc'));
  reader.querySelector('.reader-close').click();
  await wait(() => !document.querySelector('dialog.reader'), 'reader closed');
  out.afterClose = !!document.querySelector('dialog.detail[open]');
  // Escape closes only the Reader.
  sheet.querySelector('.ticket-name button').click();
  const second = await wait(() => document.querySelector('dialog.reader[open]'), 'second reader');
  await wait(() => second.querySelector('.reader-doc h2'), 'ticket document');
  out.ticketName = text(second.querySelector('h2'));
  // Headless Chromium takes no key presses from the page, so this closes the topmost dialog the way Escape does.
  second.close();
  await wait(() => !document.querySelector('dialog.reader'), 'escape closed');
  out.afterEscape = !!document.querySelector('dialog.detail[open]');
  // A document that is gone says so.
  const closeBack = document.querySelector('.detail-close'); closeBack.click();
  await wait(() => !document.querySelector('dialog.detail'), 'sheet closed');
  const gone = await rowOf('gone');
  gone.querySelector('[data-run-key]').click();
  const sheet2 = await wait(() => document.querySelector('dialog.detail[open]'), 'sheet 2');
  views(sheet2.querySelector('.status-row'))[0].click();
  const r3 = await wait(() => document.querySelector('dialog.reader[open]'), 'reader 3');
  await wait(() => !/Loading/.test(text(r3)), 'loaded');
  out.gone = text(r3);
  post(out);
})().catch((e) => post({error: String(e)}));
</script>"""


class ReaderTest(unittest.TestCase):
    def test_reader(self):
        result = run_dashboard([dict(LOCAL), dict(WEB), dict(GONE)], CHECK, handler=DocumentHandler)
        self.assertNotIn("error", result)
        self.assertEqual(result["webRowLink"], HTTP_SPEC)
        self.assertEqual(result["webRowViews"], 0)
        self.assertEqual((result["rowViews"], result["rowSpecLinks"]), (1, 0))
        self.assertEqual(result["sheetViews"], 1)
        self.assertEqual(result["titleLinks"], ["Local ticket"])
        self.assertEqual(result["webTicketLinks"], ["Ticket #231"])
        self.assertEqual(result["specName"], "spec.md")
        self.assertEqual(result["specBody"], "The specSome text.")
        self.assertTrue(result["afterClose"])
        self.assertEqual(result["ticketName"], "ticket-1.md")
        self.assertTrue(result["afterEscape"])
        self.assertIn("This document is no longer available (docs/old.md)", result["gone"])


if __name__ == "__main__":
    unittest.main()
