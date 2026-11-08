#!/usr/bin/env python3
"""Exercise the run details sheet (the route card) in Chromium; run with python3 tests/dashboard-run-details.py."""
import json
import os
import re
import unittest

from dashboard_browser import run_dashboard

AT = 1794081806000  # 2026-11-07 20:03:26 UTC
LANG = ["--lang=en-GB", "--accept-lang=en-GB"]
ENV = {**os.environ, "TZ": "Europe/Amsterdam", "LANGUAGE": "en_GB:en", "LC_ALL": "en_GB.UTF-8"}
PR = "https://github.com/acme/shop/pull/233"
SPEC = "https://github.com/acme/shop/issues/230"
T1 = "https://github.com/acme/shop/issues/231"
T2 = "https://github.com/acme/shop/issues/232"
NOTE = "Skills and activity are reported by the agent and may be incomplete."


def row(stage, model, fresh, **extra):
    return {"stage": stage, "model": model, "input": fresh - 100, "cache_read": 5000, "cache_write": 40, "output": 60, **extra}


SHIPPED = {
    "id": "run-shipped", "title": "Add a gift card checkout", "project": "/work/shop", "repository": "/work/shop",
    "worktree": "/work/shop.worktrees/gift-card-checkout", "base_commit": "9f3c2b7d1e", "branch": "gift-card-checkout",
    "mode": "balanced", "stage": "finish", "status": "done", "updated_at": AT, "activity": "Pull request merged",
    "spec": SPEC, "pr": PR, "pr_status": {"url": PR, "state": "merged", "checked_at": AT},
    "rework": 1, "current_ticket": None,
    "summaries": {"define": "Gift cards are bought at checkout and redeemed once.", "check": "One review finding fixed."},
    "tickets": [{"reference": T1, "title": "Redeem a card", "status": "done"}, {"reference": T2, "title": "Buy a card", "status": "done"}],
    "ticket_reports": [
        {"ticket": T1, "kind": "skill", "text": "factory-tdd", "stage": "build", "at": AT},
        {"ticket": T1, "kind": "activity", "text": "Redeem path covered by tests", "stage": "build", "at": AT},
    ],
    "run_reports": [
        {"kind": "skill", "text": "factory-grilling", "stage": "define", "at": AT},
        {"kind": "activity", "text": "Agreed the scope with the user", "stage": "define", "at": AT},
        {"kind": "skill", "text": "factory-implement-balanced", "stage": "build", "at": AT},
        {"kind": "activity", "text": "Checkout form built", "stage": "build", "at": AT},
        {"kind": "skill", "text": "factory-code-review", "stage": "check", "at": AT},
    ],
    # Costs come from the server; gpt-x has no price, so its cost is null.
    "usage": {"status": "tracked", "rows": [
        row("define", "opus", 1500, cost_usd=0.5), row("build", "sonnet", 20000, cost_usd=0.25),
        row("build", "sonnet", 3000, ticket=T1, cost_usd=0.05), row("build", "gpt-x", 1000, cost_usd=None),
        row("check", "gpt-x", 4000, cost_usd=None),
    ], "cost": {
        "usd": 0.8, "partial": True, "unpriced_models": ["gpt-x"],
        "by_stage": {"define": {"usd": 0.5, "partial": False}, "build": {"usd": 0.3, "partial": True, "unpriced_models": ["gpt-x"]}, "check": {"usd": None, "partial": False, "unpriced_models": ["gpt-x"]}},
        "by_model": {"opus": {"usd": 0.5, "partial": False}, "sonnet": {"usd": 0.3, "partial": False}, "gpt-x": {"usd": None, "partial": False}},
    }},
}
ACTIVE = {
    "id": "run-active", "title": "Speed up the search index", "project": "/work/shop", "repository": "/work/shop",
    "branch": "search-index", "mode": "fast", "stage": "build", "status": "running", "updated_at": AT, "activity": "Writing the index tests",
    "tickets": [{"reference": "local-1", "title": "Index in the background", "status": "in-progress"}],
    "run_reports": [{"kind": "activity", "text": "Index builder started", "stage": "build", "at": AT}],
    "usage": {"status": "tracked", "rows": [row("define", "opus", 900), row("build", "sonnet", 2100)]},
}
BLOCKED = {
    "id": "run-blocked", "title": "Migrate the orders table", "project": "/work/shop", "stage": "check", "status": "blocked",
    "updated_at": AT, "activity": "Waiting for a decision",
    "pr": PR, "pr_status": {"url": PR, "state": "unknown", "reason": "gh is not signed in", "checked_at": AT},
    "blocking_reason": "The migration drops a column that the reporting job still reads; confirm it can go before the Run continues.",
}
STRAY = {
    "id": "run-stray", "title": "Tidy the cart", "project": "/work/shop", "stage": "build", "status": "running", "updated_at": AT,
    "activity": "Working", "current_ticket": "local-9",
    "pr": PR, "pr_status": {"url": PR, "state": "unknown", "reason": "gh is not signed in", "checked_at": AT},
    "tickets": [{"reference": "local-1", "title": "First", "status": "done"}, {"reference": "local-2", "title": "Second", "status": "pending"}],
    "run_reports": [
        {"kind": "activity", "text": "No stage given", "at": AT},
        {"kind": "skill", "text": "odd-skill", "stage": "ship", "at": AT},
        {"kind": "activity", "text": "Normal build note", "stage": "build", "at": AT},
    ],
    "ticket_reports": [{"ticket": "local-404", "kind": "activity", "text": "Orphan ticket note", "stage": "build", "at": AT}],
}
CURRENT = {**ACTIVE, "id": "run-current", "title": "Current one", "current_ticket": "local-1",
           "tickets": [{"reference": "local-1", "title": "Index in the background", "status": "pending"},
                       {"reference": "local-2", "title": "Other", "status": "pending"}]}
NO_CURRENT = {**{k: v for k, v in ACTIVE.items() if k != "current_ticket"}, "id": "run-no-current", "title": "No current one",
              "tickets": [{"title": "Unreferenced work", "status": "pending"}]}
RUNS = [SHIPPED, ACTIVE, BLOCKED, STRAY, CURRENT, NO_CURRENT]

# Opens each Run's sheet in turn like a visitor and reports what it shows.
HELPERS = """
const wait = (ok, what) => new Promise((resolve, reject) => {
  const started = Date.now();
  const t = setInterval(() => { const v = ok(); if (v) { clearInterval(t); resolve(v); } else if (Date.now() - started > PAGE_WAIT_MS) { clearInterval(t); reject(new Error('timed out waiting for ' + what)); } }, 20);
});
const text = (el) => (el ? el.textContent.replace(/\\s+/g, ' ').trim() : null);
const openRun = async (id) => {
  const button = await wait(() => [...document.querySelectorAll('[data-run-key]')].find((el) => el.dataset.runKey.endsWith('\\n' + id)), 'the row of ' + id);
  button.click();
  return wait(() => document.querySelector('dialog.detail[open]'), 'the sheet of ' + id);
};
const closeRun = async () => {
  document.querySelector('.detail-close').click();
  await wait(() => !document.querySelector('dialog.detail'), 'the sheet to close');
};
const post = (data) => fetch('/result', {method: 'POST', body: JSON.stringify(data)});
const run = (work) => work().then(post, (error) => post({error: String(error.stack || error)}));
"""


def check(body):
    return f"<script>{HELPERS}\nrun(async () => {{\n{body}\n}});</script>"


def drive(body, runs=RUNS, **options):
    result = run_dashboard(json.loads(json.dumps(runs)), check(body), **{"env": ENV, "extra_args": LANG, **options})
    if "error" in result:
        raise AssertionError(result["error"])
    return result


class RunDetailsTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.shipped, cls.active, cls.blocked, cls.stray, cls.current, cls.no_current = cls.snapshots()

    @staticmethod
    def snapshots():
        body = """
const snap = async (id) => {
  const dialog = await openRun(id);
  const focusable = [...dialog.querySelectorAll('button, a[href], summary, input')];
  const amber = [...dialog.querySelectorAll('*')].filter((el) => getComputedStyle(el).backgroundColor === 'rgb(242, 183, 5)').length;
  const foot = dialog.querySelector('.detail-foot').getBoundingClientRect();
  const del = dialog.querySelector('.detail-delete').getBoundingClientRect();
  const order = (selector) => [...dialog.querySelectorAll('*')].indexOf(dialog.querySelector(selector));
  const out = {
    text: text(dialog),
    label: text(document.getElementById(dialog.getAttribute('aria-labelledby'))),
    focused: text(document.activeElement),
    firstFocusable: text(focusable[0]),
    bar: [...dialog.querySelectorAll('.detail-bar button')].map(text),
    deleteInBar: !!dialog.querySelector('.detail-bar .detail-delete'),
    deleteBottomRight: Math.abs(foot.right - del.right) < 2 && del.bottom <= foot.bottom + 1 && foot.bottom - del.bottom < 40,
    deleteLast: dialog.querySelector('.detail-foot').lastElementChild.contains(dialog.querySelector('.detail-delete')),
    heading: text(dialog.querySelector('h2')),
    sub: text(dialog.querySelector('.detail-sub')),
    activity: text(dialog.querySelector('.detail-activity')),
    statusRow: text(dialog.querySelector('.status-row')),
    otherReports: text(dialog.querySelector('.other-reports')),
    detailTicket: text(dialog.querySelector('.detail-ticket')),
    prReason: text(dialog.querySelector('.pr-reason')),
    prReasonColor: dialog.querySelector('.pr-reason') ? getComputedStyle(dialog.querySelector('.pr-reason')).color : null,
    statusRowColor: getComputedStyle(dialog.querySelector('.status-row')).color,
    statusRowBlocked: dialog.querySelector('.status-row').classList.contains('blocked'),
    amber,
    statusBeforeRepository: order('.status-row') < order('.ids'),
    prBeforeRepository: order('.status-row a[href*="/pull/"]') < order('.ids'),
    prLink: text(dialog.querySelector('.status-row a[href*="/pull/"]')),
    specLink: text(dialog.querySelector('.status-row a[href*="/issues/"]')),
    stops: [...dialog.querySelectorAll('.route > li')].map((li) => ({
      name: li.querySelector('h3').firstChild.textContent,
      state: text(li.querySelector('h3 .visually-hidden')),
      cls: li.className,
      full: text(li),
      tokens: text(li.querySelector('.stop-tokens')),
      skills: [...li.querySelectorAll(':scope > .skills > li')].map(text),
      reports: [...li.querySelectorAll(':scope > .reports li')].map(text),
      tickets: [...li.querySelectorAll(':scope > .ticket-list > li')].map(text),
      rework: text(li.querySelector(':scope > .rework')),
    })),
    routeIsList: dialog.querySelector('ol.route') !== null && dialog.querySelectorAll('ol.route > li').length === 4,
    marksHidden: [...dialog.querySelectorAll('.route .mark')].every((m) => m.getAttribute('aria-hidden') === 'true'),
    usageOpen: dialog.querySelector('details.usage')?.open,
    usageSummary: text(dialog.querySelector('details.usage summary')),
    usageTable: !!dialog.querySelector('details.usage table.usage-table'),
    rowCosts: [...dialog.querySelectorAll('details.usage table.usage-table:not(.model-table) tbody tr')].map((tr) => text(tr.lastElementChild)),
    totalCost: text(dialog.querySelector('details.usage table.usage-table:not(.model-table) tfoot td:last-child')),
    modelRows: [...dialog.querySelectorAll('details.usage table.model-table tbody tr')].map((tr) => [...tr.children].map(text)),
    modelHeader: [...dialog.querySelectorAll('details.usage table.model-table thead th')].map(text),
    stopCosts: [...dialog.querySelectorAll('.route > li')].map((li) => text(li.querySelector('.stop-cost'))),
    notes: (dialog.textContent.match(/may be incomplete/g) || []).length,
    noteInFooter: text(dialog.querySelector('.detail-foot .note')),
    times: [...dialog.querySelectorAll('time')].map((t) => ({text: t.textContent, at: t.getAttribute('datetime')})),
    copyNames: [...dialog.querySelectorAll('.copy button')].map(text),
  };
  await closeRun();
  return out;
};
return {shipped: await snap('run-shipped'), active: await snap('run-active'), blocked: await snap('run-blocked'), stray: await snap('run-stray'), current: await snap('run-current'), noCurrent: await snap('run-no-current')};
"""
        result = drive(body)
        return result["shipped"], result["active"], result["blocked"], result["stray"], result["current"], result["noCurrent"]

    def stop(self, snapshot, name):
        return next(s for s in snapshot["stops"] if s["name"] == name)

    def test_header_leads_with_status_pr_and_spec(self):
        s = self.shipped
        self.assertEqual(s["heading"], "Add a gift card checkout")
        self.assertIn("Shipped", s["statusRow"])
        self.assertEqual(s["prLink"], "PR #233")
        self.assertIn("PR #233 merged", s["statusRow"])
        self.assertEqual(s["specLink"], "Spec #230")
        self.assertTrue(s["statusBeforeRepository"] and s["prBeforeRepository"])
        self.assertIn("shop", s["sub"])
        self.assertIn("balanced", s["sub"])
        self.assertIn("gift-card-checkout", s["sub"])
        self.assertIn("Rework 1/3", s["statusRow"])
        self.assertNotIn("Rework", self.active["statusRow"])

    def test_worktree_and_base_commit_are_not_rendered(self):
        for word in ["shop.worktrees", "9f3c2b7d1e", "Base commit", "Worktree"]:
            self.assertNotIn(word, self.shipped["text"])

    def test_agent_note_appears_once_in_the_footer(self):
        for snapshot in (self.shipped, self.active, self.blocked):
            self.assertEqual(snapshot["notes"], 1)
            self.assertEqual(snapshot["noteInFooter"], NOTE)

    def test_top_bar_holds_only_close_and_delete_sits_bottom_right(self):
        for snapshot in (self.shipped, self.active, self.blocked):
            self.assertEqual(snapshot["bar"], ["CloseBack"])
            self.assertFalse(snapshot["deleteInBar"])
            self.assertTrue(snapshot["deleteBottomRight"])
            self.assertTrue(snapshot["deleteLast"])

    def test_focus_starts_on_close_and_the_dialog_is_labelled(self):
        for snapshot, title in ((self.shipped, "Add a gift card checkout"), (self.blocked, "Migrate the orders table")):
            self.assertEqual(snapshot["firstFocusable"], "CloseBack")
            self.assertEqual(snapshot["focused"], "CloseBack")
            self.assertEqual(snapshot["label"], title)

    def test_token_usage_is_open_while_active_and_collapsed_when_shipped(self):
        self.assertTrue(self.active["usageOpen"])
        self.assertFalse(self.shipped["usageOpen"])
        self.assertTrue(self.shipped["usageTable"])
        # 1,500 + 20,000 + 3,000 + 1,000 + 4,000 fresh tokens, summed by hand.
        self.assertIn("29,500 fresh", self.shipped["usageSummary"])
        self.assertIn("3,000 fresh", self.active["usageSummary"])
        self.assertNotIn("Token usage", self.blocked["text"])

    def test_estimated_cost_is_shown_per_row_model_stage_and_run_with_unknown_as_text(self):
        # Costs as the server computed them: define $0.50, build $0.25 + $0.05 + one unpriced row, check unpriced.
        self.assertEqual(self.shipped["rowCosts"], ["$0.50", "$0.25", "$0.05", "unknown", "unknown"])
        self.assertEqual(self.shipped["totalCost"], ">= $0.80 (partial), 1 unpriced: gpt-x")
        self.assertIn("Est. cost >= $0.80 (partial), 1 unpriced: gpt-x", self.shipped["usageSummary"])
        self.assertIn("29,500 fresh", self.shipped["usageSummary"])
        self.assertEqual(self.shipped["modelHeader"], ["Model", "Fresh", "Cache read", "Est. cost"])
        # Cache read per model is summed by hand: 5,000 per row.
        self.assertEqual(
            self.shipped["modelRows"],
            [["gpt-x", "5,000", "10,000", "unknown"], ["opus", "1,500", "5,000", "$0.50"], ["sonnet", "23,000", "10,000", "$0.30"]],
        )
        self.assertEqual(self.shipped["stopCosts"], ["Estimated cost $0.50", "Estimated cost >= $0.30 (partial), 1 unpriced: gpt-x", "Estimated cost unknown, 1 unpriced: gpt-x", None])
        # A Run whose usage carries no cost shows no cost summary.
        self.assertNotIn("Est. cost", self.active["usageSummary"])

    def test_reports_and_tickets_sit_under_their_stage(self):
        define, build, check, finish = (self.stop(self.shipped, n) for n in ("Define", "Build", "Check", "Finish"))
        self.assertEqual(define["skills"], ["factory-grilling"])
        self.assertTrue(define["reports"][0].endswith("Agreed the scope with the user"))
        self.assertIn("Gift cards are bought at checkout", define["full"])
        self.assertEqual(build["skills"], ["factory-implement-balanced"])
        self.assertTrue(build["reports"][0].endswith("Checkout form built"))
        self.assertEqual(len(build["tickets"]), 2)
        self.assertTrue(any("Redeem a card" in t and "factory-tdd" in t and "Redeem path covered by tests" in t for t in build["tickets"]))
        self.assertTrue(any("Buy a card" in t and "Done" in t for t in build["tickets"]))
        self.assertEqual(check["skills"], ["factory-code-review"])
        self.assertEqual(check["tickets"] + define["tickets"] + finish["tickets"], [])
        self.assertNotIn("factory-tdd", define["full"] + check["full"] + finish["full"])

    def test_unplaceable_reports_show_in_other_reports_only_when_present(self):
        other = self.stray["otherReports"]
        for shown in ("Other reports", "No stage given", "odd-skill", "Orphan ticket note"):
            self.assertIn(shown, other)
        self.assertNotIn("Normal build note", other)
        self.assertIn("Normal build note", self.stop(self.stray, "Build")["full"])
        for snapshot in (self.shipped, self.active, self.blocked, self.current):
            self.assertIsNone(snapshot["otherReports"])

    def test_current_ticket_is_marked_in_progress_in_the_ticket_list(self):
        tickets = self.stop(self.current, "Build")["tickets"]
        self.assertTrue(tickets[0].startswith("In progress"))
        self.assertTrue(tickets[1].startswith("Pending"))
        self.assertIsNone(self.current["detailTicket"])

    def test_pending_ticket_without_reference_is_not_in_progress_when_the_run_has_no_current_ticket(self):
        tickets = self.stop(self.no_current, "Build")["tickets"]
        self.assertEqual(len(tickets), 1)
        self.assertTrue(tickets[0].startswith("Pending"))

    def test_current_ticket_missing_from_the_list_shows_under_the_title(self):
        self.assertEqual(self.stray["detailTicket"], "Current ticket: local-9")
        self.assertIsNone(self.active["detailTicket"])

    def test_unknown_pr_state_reason_is_visible_text(self):
        self.assertIn("state unknown: gh is not signed in", self.stray["statusRow"])
        self.assertEqual(self.stray["prReason"], "gh is not signed in")
        self.assertIsNone(self.shipped["prReason"])

    def test_pr_reason_on_the_amber_row_uses_the_on_amber_text_colour(self):
        self.assertEqual(self.blocked["prReason"], "gh is not signed in")
        self.assertEqual(self.blocked["prReasonColor"], self.blocked["statusRowColor"])

    def test_build_shows_the_ticket_progress_count(self):
        self.assertIn("2 of 2 tickets done", self.stop(self.shipped, "Build")["full"])
        self.assertIn("1 of 2 tickets done", self.stop(self.stray, "Build")["full"])
        self.assertNotIn("tickets done", self.stop(self.blocked, "Build")["full"])

    def test_rework_marker_is_on_check_only_when_rework_is_above_zero(self):
        self.assertEqual(self.stop(self.shipped, "Check")["rework"], "Rework 1/3")
        for name in ("Define", "Build", "Finish"):
            self.assertIsNone(self.stop(self.shipped, name)["rework"])
        for stop in self.active["stops"] + self.blocked["stops"]:
            self.assertIsNone(stop["rework"])

    def test_stations_show_their_fresh_tokens_and_a_bare_name_without_data(self):
        shipped = {s["name"]: s for s in self.shipped["stops"]}
        self.assertEqual(shipped["Define"]["tokens"], "1.5k fresh tokens")
        self.assertEqual(shipped["Build"]["tokens"], "24.0k fresh tokens")
        self.assertEqual(shipped["Check"]["tokens"], "4.0k fresh tokens")
        self.assertIsNone(shipped["Finish"]["tokens"])
        self.assertEqual(shipped["Finish"]["full"], "Finish, done")
        blocked = {s["name"]: s for s in self.blocked["stops"]}
        for name in ("Define", "Build", "Check", "Finish"):
            self.assertIsNone(blocked[name]["tokens"])
            self.assertEqual(blocked[name]["full"].split(",")[0], name)
            self.assertEqual(len(blocked[name]["full"].split(",")), 2 if name != "Check" else 3)

    def test_rail_is_a_list_with_stage_names_and_states_as_text(self):
        for snapshot in (self.shipped, self.active, self.blocked):
            self.assertTrue(snapshot["routeIsList"])
            self.assertTrue(snapshot["marksHidden"])
            self.assertEqual([s["name"] for s in snapshot["stops"]], ["Define", "Build", "Check", "Finish"])
        self.assertEqual([s["state"] for s in self.shipped["stops"]], [", done"] * 4)
        self.assertEqual([s["state"] for s in self.active["stops"]], [", done", ", current", ", not reached", ", not reached"])
        self.assertEqual([s["state"] for s in self.blocked["stops"]], [", done", ", done", ", current, blocked", ", not reached"])
        self.assertIn("current", self.active["stops"][1]["cls"])
        self.assertIn("blocked", self.blocked["stops"][2]["cls"])

    def test_blocked_run_shows_an_amber_row_with_the_full_reason_and_others_show_no_amber(self):
        self.assertTrue(self.blocked["statusRowBlocked"])
        self.assertIn(BLOCKED["blocking_reason"], self.blocked["statusRow"])
        self.assertGreater(self.blocked["amber"], 0)
        for snapshot in (self.shipped, self.active):
            self.assertFalse(snapshot["statusRowBlocked"])
            self.assertEqual(snapshot["amber"], 0)

    def test_activity_shows_under_the_title_only_while_active(self):
        self.assertEqual(self.active["activity"], "Writing the index tests")
        self.assertEqual(self.blocked["activity"], "Waiting for a decision")
        self.assertIsNone(self.shipped["activity"])
        self.assertNotIn("Pull request merged", self.shipped["text"])

    def test_empty_fields_are_not_rendered(self):
        for snapshot in (self.shipped, self.active, self.blocked):
            self.assertNotIn("Unknown", snapshot["text"])
            self.assertNotIn("Not set", snapshot["text"])
        self.assertNotIn("Repository", self.blocked["text"])
        self.assertEqual(self.blocked["copyNames"], ["Copy run id"])

    def test_every_time_uses_the_single_format(self):
        pattern = re.compile(r"^(\d{2}:\d{2}|Yesterday \d{2}:\d{2}|[A-Z][a-z]{2} \d{1,2} [A-Z][a-z]{2} \d{2}:\d{2})$")
        # Updated, the run reports and the ticket report of the shipped run; Updated and one report of the active run.
        self.assertEqual(len(self.shipped["times"]), 4)
        self.assertEqual(len(self.active["times"]), 2)
        for snapshot in (self.shipped, self.active):
            for shown in snapshot["times"]:
                self.assertRegex(shown["text"], pattern)
                self.assertEqual(shown["at"], "2026-11-07T20:03:26.000Z")
                self.assertTrue(shown["text"].endswith("21:03"), shown)
        self.assertNotRegex(self.shipped["text"], r"\d{2}/\d{2}/\d{4}|ago")

    def test_copy_buttons_have_names_and_announce_copied(self):
        self.assertEqual(self.shipped["copyNames"], ["Copy branch", "Copy repository", "Copy run id"])
        body = """
// The browser's clipboard needs a real user gesture; stand in for it and record what would be copied.
const copied = [];
Object.defineProperty(navigator, 'clipboard', {value: {writeText: async (value) => { copied.push(value); }}});
const dialog = await openRun('run-shipped');
const announced = [];
const live = dialog.querySelector('.copy [role="status"]');
const watcher = new MutationObserver(() => announced.push(text(live)));
watcher.observe(live, {childList: true, characterData: true, subtree: true});
dialog.querySelector('.copy button').click();
await wait(() => text(live) === 'Copied branch', 'the Copied announcement');
return {announced, copied, button: text(dialog.querySelector('.copy button'))};
"""
        result = drive(body)
        self.assertEqual(result["button"], "Copied branch")
        self.assertIn("Copied branch", result["announced"])
        self.assertEqual(result["copied"], ["gift-card-checkout"])

    def test_delete_run_still_asks_before_deleting(self):
        body = """
const dialog = await openRun('run-blocked');
dialog.querySelector('.detail-delete').click();
const confirm = await wait(() => document.querySelector('dialog.confirm[open]'), 'the confirm dialog');
const out = {title: text(confirm.querySelector('h2')), body: text(confirm), opened: true};
confirm.querySelector('button:not(.danger)').click();
await wait(() => !document.querySelector('dialog.confirm'), 'the confirm dialog to close');
out.stillThere = !!document.querySelector('dialog.detail[open]') || [...document.querySelectorAll('[data-run-key]')].some((el) => el.dataset.runKey.endsWith('run-blocked'));
return out;
"""
        result = drive(body)
        self.assertEqual(result["title"], "Delete this run?")
        self.assertIn("Migrate the orders table", result["body"])
        self.assertTrue(result["stillThere"])

    def test_time_format_for_today_yesterday_and_older(self):
        body = """
const {sheetTime} = await import('/assets/app.js');
const now = new Date(2026, 10, 8, 12, 0).getTime();
return {
  today: sheetTime(new Date(2026, 10, 8, 9, 22).getTime(), now),
  yesterday: sheetTime(new Date(2026, 10, 7, 16, 40).getTime(), now),
  older: sheetTime(new Date(2026, 10, 6, 14, 2).getTime(), now),
  lateYesterday: sheetTime(new Date(2026, 10, 7, 23, 59).getTime(), new Date(2026, 10, 8, 0, 1).getTime()),
};
"""
        result = drive(body)
        self.assertEqual(result["today"], "09:22")
        self.assertEqual(result["yesterday"], "Yesterday 16:40")
        self.assertEqual(result["older"], "Fri 6 Nov 14:02")
        self.assertEqual(result["lateYesterday"], "Yesterday 23:59")

    CONTRAST = """
const dialog = await openRun('run-shipped');
const rgb = (value) => value.match(/[\\d.]+/g).slice(0, 3).map(Number);
const lum = (c) => c.map((v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }).reduce((a, v, i) => a + v * [0.2126, 0.7152, 0.0722][i], 0);
const ratio = (a, b) => { const [x, y] = [lum(rgb(a)), lum(rgb(b))].sort((p, q) => q - p); return (x + 0.05) / (y + 0.05); };
const panel = getComputedStyle(dialog).backgroundColor;
const pick = (selector) => getComputedStyle(dialog.querySelector(selector)).color;
const mark = dialog.querySelector('.stop .mark');
return {
  panel, mark: getComputedStyle(mark).transitionDuration,
  muted: ratio(pick('.stop-tokens'), panel), body: ratio(pick('.stop-summary'), panel), skill: ratio(pick('.skill'), panel),
  link: ratio(pick('.status-row a'), panel),
  doneMark: getComputedStyle(dialog.querySelector('.stop.done .mark')).backgroundColor, panelRatio: ratio(getComputedStyle(dialog.querySelector('.stop.done .mark')).backgroundColor, panel),
};
"""

    def test_light_and_dark_themes_keep_text_readable(self):
        light = drive(self.CONTRAST, size=(1280, 900))
        dark = drive(self.CONTRAST, size=(1280, 900), extra_args=[*LANG, "--force-dark-mode", "--enable-features=WebContentsForceDark"])
        self.assertEqual(light["panel"], "rgb(255, 255, 255)")
        self.assertEqual(dark["panel"], "rgb(31, 37, 44)")
        for result in (light, dark):
            for part in ("muted", "body", "skill", "link"):
                self.assertGreaterEqual(result[part], 4.5, (part, result))
            self.assertGreaterEqual(result["panelRatio"], 3, result)

    def test_reduced_motion_disables_the_mark_transition(self):
        normal = drive(self.CONTRAST)
        reduced = drive(self.CONTRAST, extra_args=[*LANG, "--force-prefers-reduced-motion"])
        self.assertNotEqual(normal["mark"], "0s")
        self.assertEqual(reduced["mark"], "0s")


if __name__ == "__main__":
    unittest.main()
