#!/usr/bin/env python3
"""Exercise the run details timeline of stage passes in Chromium; run with python3 tests/dashboard-run-timeline.py."""
import json
import os
import re
import unittest

from dashboard_browser import run_dashboard

LANG = ["--lang=en-GB", "--accept-lang=en-GB"]
ENV = {**os.environ, "TZ": "Europe/Amsterdam", "LANGUAGE": "en_GB:en", "LC_ALL": "en_GB.UTF-8"}

# A run with three reworks, shaped like a real one: marks in milliseconds since the epoch, 2026-11-08 in Amsterdam.
T = {
    "define": 1794131952711, "build": 1794132006929, "check1": 1794132788265, "rework1": 1794132860161,
    "check2": 1794133137710, "rework2": 1794133171729, "check3": 1794133307982, "rework3": 1794133398092,
    "check4": 1794133551320, "finish": 1794133609836, "end": 1794133966975,
}
MARKS = [
    {"at": T["define"], "stage": "define", "activity": "Planning the route card"},
    {"at": T["build"], "stage": "build", "activity": "Building the route card"},
    {"at": T["check1"], "stage": "check", "activity": "Reviewing the change"},
    {"at": T["rework1"], "stage": "build", "rework": 1, "activity": "Fixing the first review findings"},
    {"at": T["check2"], "stage": "check"},
    {"at": T["rework2"], "stage": "build", "rework": 2, "activity": "Fixing the second review findings"},
    {"at": T["check3"], "stage": "check", "activity": "Reviewing again"},
    {"at": T["rework3"], "stage": "build", "rework": 3, "activity": "Fixing the third review findings"},
    {"at": T["check4"], "stage": "check", "activity": "Final review"},
    {"at": T["finish"], "stage": "finish", "activity": "Opening the pull request"},
]
LABELS = [
    "Define", "Build", "Check", "Build · Rework 1", "Check", "Build · Rework 2", "Check",
    "Build · Rework 3 (past the cap of 2)", "Check", "Finish",
]
# Hand-worked from the marks above: 54 s, 13 min 1 s, 1 min 12 s, 4 min 38 s, 34 s, 2 min 16 s, 1 min 30 s, 2 min 33 s, 58 s, 5 min 57 s.
DURATIONS = ["under 1 min", "13 min", "1 min", "4 min", "under 1 min", "2 min", "1 min", "2 min", "under 1 min", "5 min"]
SHIPPED = {
    "id": "run-rework", "title": "Redesign the run details", "project": "/work/shop", "stage": "finish", "status": "done",
    "rework": 3, "updated_at": T["end"], "finished_at": T["end"], "activity": "Pull request merged", "timeline": MARKS,
    "summaries": {
        "define": "Design agreed with the user.", "build": "Route card built with browser tests.",
        "check": "Three reworks; the final pass was clean.", "finish": "Pull request opened and merged.",
    },
    "tickets": [],
    "run_reports": [
        {"kind": "skill", "text": "frontend-design", "stage": "build", "at": T["build"]},
        {"kind": "activity", "text": "Plan written", "stage": "build", "at": T["build"]},
        {"kind": "skill", "text": "factory-code-review", "stage": "check", "at": T["check1"] + 5000},
        {"kind": "activity", "text": "Screenshots reviewed", "stage": "check", "at": T["check1"] + 5000},
        {"kind": "activity", "text": "Third rework approved", "stage": "finish", "at": T["check4"] + 1000},
        {"kind": "activity", "text": "Pull request opened", "stage": "finish", "at": T["end"]},
        {"kind": "activity", "text": "Noted before the run began", "at": T["define"] - 60000},
    ],
    "ticket_reports": [{"ticket": "gone", "kind": "activity", "text": "Report for an unlisted ticket", "at": T["build"]}],
}
BLOCKED = {
    "id": "run-blocked", "title": "Migrate the orders table", "project": "/work/shop", "stage": "build", "status": "blocked",
    "rework": 1, "updated_at": T["rework1"] + 600000, "blocking_reason": "Waiting for a decision on the dropped column.",
    "activity": "Waiting", "timeline": MARKS[:4],
}
ACTIVE = {
    "id": "run-active", "title": "Speed up the search index", "project": "/work/shop", "stage": "build", "status": "running",
    "updated_at": T["build"] + 5 * 60000 + 20000, "timeline": MARKS[:2], "current_ticket": "t2",
}
# Build marks that differ only by ticket make one pass; t3 has not been started.
TICKETS = {
    "id": "run-tickets", "title": "Gift cards", "project": "/work/shop", "stage": "check", "status": "running",
    "updated_at": T["check1"] + 60000, "current_ticket": None,
    "timeline": [
        {"at": T["define"], "stage": "define"},
        {"at": T["build"], "stage": "build", "activity": "Implementing the tickets"},
        {"at": T["build"] + 60000, "stage": "build", "ticket": "t1", "activity": "Implementing the tickets"},
        {"at": T["build"] + 120000, "stage": "build", "ticket": "t2", "activity": "Writing the buy path"},
        {"at": T["check1"], "stage": "check"},
    ],
    "tickets": [
        {"reference": "t1", "title": "Redeem a card", "status": "done"},
        {"reference": "t2", "title": "Buy a card", "status": "done"},
        {"reference": "t3", "title": "Email a card", "status": "pending"},
    ],
    "ticket_reports": [
        {"ticket": "t1", "kind": "activity", "text": "Redeem path covered", "at": T["build"] + 90000},
        {"ticket": "t2", "kind": "activity", "text": "Buy path covered", "at": T["build"] + 150000},
        {"ticket": "t1", "kind": "activity", "text": "Late note on redeem", "at": T["check1"] + 1000},
    ],
    "run_reports": [],
}
NO_TIMELINE = {
    "id": "run-plain", "title": "No marks yet", "project": "/work/shop", "stage": "build", "status": "running",
    "updated_at": T["end"], "run_reports": [{"kind": "activity", "text": "Index builder started", "stage": "build", "at": T["build"]}],
}
EMPTY_TIMELINE = {**NO_TIMELINE, "id": "run-empty", "timeline": []}
# The timeline was cut short: no Define or Build pass, but their summaries are kept.
TRUNCATED = {
    **SHIPPED, "id": "run-truncated", "timeline": MARKS[-2:], "run_reports": [], "ticket_reports": [],
    "summaries": {"define": "Design agreed.", "build": "Built with tests.", "finish": "Opened the pull request."},
}
# Still in Define: no Build pass exists, so the tickets are not started.
DEFINING = {
    "id": "run-defining", "title": "Plan the export", "project": "/work/shop", "stage": "define", "status": "running",
    "updated_at": T["define"] + 60000, "timeline": MARKS[:1], "run_reports": [],
    "tickets": [{"reference": "t1", "title": "Export to CSV", "status": "pending"}, {"reference": "t2", "title": "Export to PDF", "status": "pending"}],
    "ticket_reports": [{"ticket": "t1", "kind": "activity", "text": "Sketched the CSV columns", "at": T["define"] + 30000}],
}
RUNS = [SHIPPED, BLOCKED, ACTIVE, TICKETS, NO_TIMELINE, EMPTY_TIMELINE, TRUNCATED, DEFINING]

HELPERS = """
const wait = (ok, what) => new Promise((resolve, reject) => {
  const started = Date.now();
  const t = setInterval(() => { const v = ok(); if (v) { clearInterval(t); resolve(v); } else if (Date.now() - started > 15000) { clearInterval(t); reject(new Error('timed out waiting for ' + what)); } }, 20);
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
const amberCount = (root) => [...root.querySelectorAll('*')].filter((el) => getComputedStyle(el).backgroundColor === 'rgb(242, 183, 5)').length;
const snap = async (id) => {
  const dialog = await openRun(id);
  const out = {
    passes: [...dialog.querySelectorAll('.route > li')].map((li) => ({
      label: text(li.querySelector('h3').firstChild),
      state: text(li.querySelector('h3 .visually-hidden')),
      cls: li.className,
      when: text(li.querySelector('.pass-when')),
      time: li.querySelector('.pass-when time')?.getAttribute('datetime'),
      duration: text(li.querySelector('.pass-duration')),
      activity: text(li.querySelector('.pass-activity')),
      events: [...li.querySelectorAll(':scope > .pass-event')].map(text),
      activities: [...li.querySelectorAll(':scope > .pass-activity')].map(text),
      summary: text(li.querySelector('.stop-summary')),
      skills: [...li.querySelectorAll(':scope > .skills > li')].map(text),
      reports: [...li.querySelectorAll(':scope > .reports li')].map(text),
      tickets: [...li.querySelectorAll(':scope > .ticket-list > li')].map(text),
      count: text(li.querySelector('.ticket-count')),
      tokens: text(li.querySelector('.stop-tokens')),
      sentBack: text(li.querySelector('.sent-back-note')),
      amber: amberCount(li),
      markHidden: li.querySelector('.mark').getAttribute('aria-hidden'),
    })),
    other: text(dialog.querySelector('.other-reports')),
    unstarted: [...dialog.querySelectorAll('.unstarted-tickets > .ticket-list > li')].map(text),
    unstartedHeading: text(dialog.querySelector('.unstarted-tickets h3')),
    unstartedAfterRail: !!dialog.querySelector('.route + .unstarted-tickets'),
    routeIsList: dialog.querySelector('ol.route') !== null,
    statusRowAmber: amberCount(dialog.querySelector('.status-row')) + (getComputedStyle(dialog.querySelector('.status-row')).backgroundColor === 'rgb(242, 183, 5)' ? 1 : 0),
    amber: amberCount(dialog),
  };
  await closeRun();
  return out;
};
"""


def drive(body, runs=RUNS, **options):
    page = f"<script>{HELPERS}\nrun(async () => {{\n{body}\n}});</script>"
    result = run_dashboard(json.loads(json.dumps(runs)), page, **{"env": ENV, "extra_args": LANG, **options})
    if "error" in result:
        raise AssertionError(result["error"])
    return result


class RunTimelineTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        ids = ["run-rework", "run-blocked", "run-active", "run-tickets", "run-plain", "run-empty", "run-truncated", "run-defining"]
        body = "return {" + ", ".join(f"'{i}': await snap('{i}')" for i in ids) + "};"
        cls.snaps = drive(body)
        cls.rework = cls.snaps["run-rework"]["passes"]

    def test_every_pass_of_a_run_with_reworks_shows_in_order(self):
        self.assertEqual([p["label"] for p in self.rework], LABELS)
        self.assertEqual(len([p for p in self.rework if p["label"].startswith("Build · Rework")]), 3)
        times = [p["time"] for p in self.rework]
        self.assertEqual(times, sorted(times))
        self.assertEqual(self.rework[0]["time"], "2026-11-08T09:59:12.711Z")
        self.assertTrue(self.snaps["run-rework"]["routeIsList"])
        self.assertTrue(all(p["markHidden"] == "true" for p in self.rework))

    def test_a_pass_started_by_a_rework_has_a_ringed_mark_class_and_the_rework_number_in_words(self):
        reworks = [p for p in self.rework if "Rework" in p["label"]]
        self.assertTrue(all("rework-pass" in p["cls"] for p in reworks))
        self.assertFalse(any("rework-pass" in p["cls"] for p in self.rework if "Rework" not in p["label"]))
        self.assertEqual([p["label"] for p in reworks][:2], ["Build · Rework 1", "Build · Rework 2"])
        self.assertIn("past the cap of 2", reworks[2]["label"])

    def test_a_check_pass_followed_by_a_rework_says_it_was_sent_back(self):
        sent = [i for i, p in enumerate(self.rework) if p["sentBack"] == "sent back"]
        self.assertEqual(sent, [2, 4, 6])
        for i in sent:
            self.assertIn("sent-back", self.rework[i]["cls"])
            self.assertTrue(self.rework[i + 1]["label"].startswith("Build · Rework"))
        self.assertEqual(sum(p["sentBack"] is not None for p in self.rework), 3)

    def test_durations_are_plain_words_until_the_next_pass_and_the_last_runs_to_the_end(self):
        self.assertEqual([p["duration"] for p in self.rework], DURATIONS)
        self.assertRegex(self.rework[1]["when"], r"^\d{2}:\d{2} · 13 min$")

    def test_the_last_pass_of_an_active_run_runs_to_now_and_a_finished_one_to_its_end(self):
        body = """
const {timelinePasses} = await import('/assets/app.js');
const run = {status: 'running', updated_at: 1000, timeline: [{at: 0, stage: 'define'}, {at: 600000, stage: 'build'}]};
const at = (now, extra = {}) => timelinePasses({...run, ...extra}, now).passes.map((p) => p.duration);
return [at(1800000), at(1800000, {status: 'done', finished_at: 900000}), at(1800000, {status: 'blocked'})];
"""
        active, done, blocked = drive(body)
        self.assertEqual(active, [600000, 1200000])
        self.assertEqual(done, [600000, 300000])
        self.assertEqual(blocked, [600000, 1200000])

    def test_the_active_pass_in_the_sheet_counts_to_the_current_time(self):
        body = """
const before = Date.now();
const shot = await snap('run-active');
const {durationWords} = await import('/assets/app.js');
const start = %d;
return {last: shot.passes[1].duration, wanted: [durationWords(before - start), durationWords(Date.now() - start)]};
""" % T["build"]
        result = drive(body)
        self.assertIn(result["last"], result["wanted"])
        self.assertNotEqual(result["last"], "5 min")

    def test_the_duration_function_uses_the_plain_word_formats(self):
        body = """
const {durationWords} = await import('/assets/app.js');
const min = 60000;
return [0, 59999, min, 12 * min + 59000, 59 * min, 60 * min, 65 * min, 125 * min, 26 * 60 * min + 3 * min, -5].map(durationWords);
"""
        self.assertEqual(
            drive(body),
            ["under 1 min", "under 1 min", "1 min", "12 min", "59 min", "1 h", "1 h 5 min", "2 h 5 min", "26 h 3 min", "under 1 min"],
        )

    def test_each_pass_shows_the_activity_it_started_with_and_only_when_it_has_one(self):
        self.assertEqual([p["activity"] for p in self.rework], [m.get("activity") for m in MARKS])
        self.assertIsNone(self.rework[4]["activity"])

    def test_each_stage_summary_appears_once_on_the_last_pass_of_its_stage(self):
        shown = [(i, p["summary"]) for i, p in enumerate(self.rework) if p["summary"]]
        self.assertEqual(shown, [
            (0, SHIPPED["summaries"]["define"]), (7, SHIPPED["summaries"]["build"]),
            (8, SHIPPED["summaries"]["check"]), (9, SHIPPED["summaries"]["finish"]),
        ])

    def test_reports_land_in_the_pass_of_their_stage(self):
        by_label = {i: p for i, p in enumerate(self.rework)}
        self.assertEqual(by_label[1]["skills"], ["frontend-design"])
        self.assertTrue(by_label[1]["reports"][0].endswith("Plan written"))
        # Filed 5 s into the first Check pass, and names Check.
        self.assertEqual(by_label[2]["skills"], ["factory-code-review"])
        self.assertTrue(by_label[2]["reports"][0].endswith("Screenshots reviewed"))
        # The first report of the last pass's open-ended window is the one at the run's end.
        self.assertTrue(by_label[9]["reports"][-1].endswith("Pull request opened"))
        # Names Finish, so it goes in the Finish pass although it was filed in the fourth Check window.
        self.assertEqual(by_label[8]["reports"], [])
        self.assertTrue(by_label[9]["reports"][0].endswith("Third rework approved"))
        placed = sum(len(p["reports"]) for p in self.rework)
        self.assertEqual(placed, 4)

    def test_a_report_goes_in_the_last_pass_of_its_stage_started_before_it_else_the_nearest_pass(self):
        # Check passes start at check1..check4; the Build ones at build and the reworks.
        wanted = {"early": 2, "late": 8, "rework": 7, "nostart": 1}
        runs = [{
            **SHIPPED, "id": "run-stage", "ticket_reports": [],
            "run_reports": [
                {"kind": "activity", "text": "early", "stage": "check", "at": T["check1"] + 1},
                {"kind": "activity", "text": "late", "stage": "check", "at": T["end"]},
                {"kind": "activity", "text": "rework", "stage": "build", "at": T["finish"]},
                {"kind": "activity", "text": "nostart", "stage": "build", "at": T["define"]},
            ],
        }]
        body = "return await snap('run-stage');"
        passes = drive(body, runs=runs)["passes"]
        for text, index in wanted.items():
            self.assertEqual([i for i, p in enumerate(passes) if any(r.endswith(text) for r in p["reports"])], [index], text)

    def test_ticket_reports_follow_their_stage_and_go_under_the_ticket_only_when_the_pass_lists_it(self):
        runs = [{
            **TICKETS, "id": "run-ticket-stage",
            "ticket_reports": [
                {"ticket": "t3", "kind": "activity", "text": "Check noted t3", "stage": "check", "at": T["build"] + 1000},
                {"ticket": "t3", "kind": "activity", "text": "Build noted t3", "stage": "build", "at": T["check1"] + 5000},
                {"ticket": "t1", "kind": "activity", "text": "Check noted t1", "stage": "check", "at": T["build"] + 2000},
            ],
        }]
        passes = drive("return await snap('run-ticket-stage');", runs=runs)["passes"]
        build, check = passes[1], passes[2]
        self.assertTrue(any("Check noted t3" in r for r in check["reports"]))
        self.assertTrue(any("Check noted t1" in r for r in check["reports"]))
        self.assertIn("Build noted t3", build["tickets"][2])
        self.assertNotIn("Check noted", " ".join(build["tickets"]))

    def test_blocked_and_resumed_events_show_as_lines_in_the_pass_that_holds_them(self):
        runs = [{
            **BLOCKED, "id": "run-events", "status": "running", "stage": "check", "blocking_reason": None,
            "timeline": MARKS[:3],
            "events": [
                {"at": T["build"] + 60000, "kind": "blocked", "stage": "build", "reason": "Waiting for a decision."},
                {"at": T["build"] + 120000, "kind": "resumed", "stage": "build"},
                {"at": T["check1"] + 1000, "kind": "blocked", "stage": "check"},
            ],
        }]
        passes = drive("return await snap('run-events');", runs=runs)["passes"]
        self.assertEqual([p["events"] for p in passes[:1]], [[]])
        self.assertEqual(len(passes[1]["events"]), 2)
        self.assertTrue(passes[1]["events"][0].startswith("Blocked: Waiting for a decision."))
        self.assertTrue(passes[1]["events"][1].startswith("Resumed"))
        self.assertEqual(len(passes[2]["events"]), 1)
        self.assertTrue(passes[2]["events"][0].startswith("Blocked"))
        # A run without events shows no event lines.
        self.assertEqual(self.snaps["run-rework"]["passes"][1]["events"], [])

    def test_events_that_match_no_pass_show_under_other_reports(self):
        runs = [
            {**BLOCKED, "id": "run-events-define", "timeline": MARKS[:3], "events": [
                {"at": T["define"] - 5000, "kind": "blocked", "stage": "finish", "reason": "Needs an answer."},
                {"at": T["define"] - 4000, "kind": "resumed", "stage": "finish"},
            ]},
            {**EMPTY_TIMELINE, "id": "run-events-empty", "events": [{"at": T["build"], "kind": "blocked", "stage": "build", "reason": "No timeline."}]},
        ]
        snaps = drive("return {a: await snap('run-events-define'), b: await snap('run-events-empty')};", runs=runs)
        self.assertIn("Blocked: Needs an answer.", snaps["a"]["other"])
        self.assertIn("Resumed", snaps["a"]["other"])
        self.assertIn("Blocked: No timeline.", snaps["b"]["other"])

    def test_reports_before_the_first_mark_and_for_unlisted_tickets_go_to_other_reports(self):
        other = self.snaps["run-rework"]["other"]
        self.assertIn("Other reports", other)
        # No stage (older data) and before the first pass: no window holds it.
        self.assertIn("Noted before the run began", other)
        self.assertIn("Report for an unlisted ticket", other)
        self.assertNotIn("Plan written", other)
        for key in ("run-blocked", "run-active", "run-tickets"):
            self.assertIsNone(self.snaps[key]["other"])

    def test_consecutive_build_marks_that_differ_by_ticket_form_one_pass_listing_the_tickets(self):
        passes = self.snaps["run-tickets"]["passes"]
        self.assertEqual([p["label"] for p in passes], ["Define", "Build", "Check"])
        build = passes[1]
        self.assertEqual(len(build["tickets"]), 3)
        self.assertTrue(build["tickets"][0].startswith("Done") and "Redeem a card" in build["tickets"][0])
        self.assertIn("Redeem path covered", build["tickets"][0])
        self.assertIn("Buy path covered", build["tickets"][1])
        self.assertTrue(build["tickets"][2].startswith("Pending") and "Email a card" in build["tickets"][2])
        self.assertEqual(build["count"], "2 of 3 tickets done")
        self.assertEqual(build["activity"], "Implementing the tickets")
        # A ticket report filed in the Check window shows there, not under the ticket of the Build pass.
        self.assertEqual(passes[2]["tickets"], [])
        self.assertTrue(passes[2]["reports"][0].endswith("Late note on redeem"))
        self.assertNotIn("Late note", build["tickets"][0])
        # It still says which ticket it is about.
        self.assertIn("t1 Late note on redeem", passes[2]["reports"][0])

    def test_a_merged_build_pass_shows_each_distinct_activity_in_order(self):
        build = self.snaps["run-tickets"]["passes"][1]
        self.assertEqual(build["activities"], ["Implementing the tickets", "Writing the buy path"])

    def test_summaries_of_a_stage_without_a_pass_show_under_other_reports(self):
        snapshot = self.snaps["run-truncated"]
        self.assertEqual([p["label"] for p in snapshot["passes"]], ["Check", "Finish"])
        self.assertIn("Define: Design agreed.", snapshot["other"])
        self.assertIn("Build: Built with tests.", snapshot["other"])
        self.assertNotIn("Check:", snapshot["other"])
        self.assertEqual(snapshot["passes"][1]["summary"], "Opened the pull request.")
        self.assertIsNone(snapshot["passes"][0]["summary"])

    def test_tickets_without_a_first_build_pass_show_in_a_tickets_group_after_the_rail(self):
        snapshot = self.snaps["run-defining"]
        self.assertEqual([p["label"] for p in snapshot["passes"]], ["Define"])
        self.assertEqual(snapshot["passes"][0]["tickets"], [])
        self.assertEqual(snapshot["unstartedHeading"], "Tickets")
        self.assertTrue(snapshot["unstartedAfterRail"])
        self.assertEqual(len(snapshot["unstarted"]), 2)
        self.assertTrue(snapshot["unstarted"][0].startswith("Pending") and "Export to CSV" in snapshot["unstarted"][0])
        self.assertIn("Sketched the CSV columns", snapshot["unstarted"][0])
        self.assertIsNone(snapshot["other"])
        # With a Build pass the tickets stay in it and no group appears.
        self.assertEqual(self.snaps["run-tickets"]["unstarted"], [])

    def test_pass_state_is_text_and_per_pass_tokens_are_gone(self):
        self.assertEqual({p["state"] for p in self.rework}, {", done"})
        active = self.snaps["run-active"]["passes"]
        self.assertEqual([p["state"] for p in active], [", done", ", current"])
        self.assertTrue(all(p["tokens"] is None for p in self.rework))

    def test_amber_marks_only_the_blocked_current_pass_and_the_blocked_status_row(self):
        passes = self.snaps["run-blocked"]["passes"]
        self.assertEqual([p["label"] for p in passes], ["Define", "Build", "Check", "Build · Rework 1"])
        self.assertEqual([p["state"] for p in passes], [", done", ", done", ", done", ", current, blocked"])
        self.assertEqual([p["amber"] > 0 for p in passes], [False, False, False, True])
        self.assertIn("blocked", passes[3]["cls"])
        self.assertEqual(self.snaps["run-blocked"]["amber"], sum(p["amber"] for p in passes) + 1)
        for key in ("run-rework", "run-active", "run-tickets"):
            self.assertEqual(self.snaps[key]["amber"], 0)

    def test_runs_without_timeline_marks_keep_the_four_stations(self):
        for key in ("run-plain", "run-empty"):
            snapshot = self.snaps[key]
            self.assertEqual([p["label"] for p in snapshot["passes"]], ["Define", "Build", "Check", "Finish"])
            self.assertEqual([p["state"] for p in snapshot["passes"]], [", done", ", current", ", not reached", ", not reached"])
            self.assertTrue(all(p["when"] is None for p in snapshot["passes"]))
            self.assertTrue(snapshot["passes"][1]["reports"][0].endswith("Index builder started"))

    def test_the_blocked_reason_is_readable_on_amber_in_both_themes(self):
        body = """
const dialog = await openRun('run-blocked');
const rgb = (value) => value.match(/[\\d.]+/g).slice(0, 3).map(Number);
const lum = (c) => c.map((v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }).reduce((a, v, i) => a + v * [0.2126, 0.7152, 0.0722][i], 0);
const row = dialog.querySelector('.status-row.blocked');
const reason = getComputedStyle(dialog.querySelector('.status-reason'));
const [x, y] = [lum(rgb(reason.color)), lum(rgb(getComputedStyle(row).backgroundColor))].sort((p, q) => q - p);
return {panel: getComputedStyle(dialog).backgroundColor, reason: reason.color, onAmber: getComputedStyle(row).color, ratio: (x + 0.05) / (y + 0.05)};
"""
        for args, panel in (([], "rgb(255, 255, 255)"), (["--force-dark-mode", "--enable-features=WebContentsForceDark"], "rgb(31, 37, 44)")):
            result = drive(body, extra_args=[*LANG, *args])
            self.assertEqual(result["panel"], panel)
            self.assertEqual(result["reason"], result["onAmber"])
            self.assertGreaterEqual(result["ratio"], 4.5, result)

    def test_the_timeline_keeps_both_themes_readable_and_respects_reduced_motion(self):
        body = """
const dialog = await openRun('run-rework');
const rgb = (value) => value.match(/[\\d.]+/g).slice(0, 3).map(Number);
const lum = (c) => c.map((v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }).reduce((a, v, i) => a + v * [0.2126, 0.7152, 0.0722][i], 0);
const ratio = (a, b) => { const [x, y] = [lum(rgb(a)), lum(rgb(b))].sort((p, q) => q - p); return (x + 0.05) / (y + 0.05); };
const panel = getComputedStyle(dialog).backgroundColor;
const pick = (selector) => getComputedStyle(dialog.querySelector(selector)).color;
return {panel, mark: getComputedStyle(dialog.querySelector('.stop .mark')).transitionDuration,
  when: ratio(pick('.pass-when'), panel), activity: ratio(pick('.pass-activity'), panel), sentBack: ratio(pick('.sent-back-note'), panel)};
"""
        light = drive(body, size=(1280, 900))
        dark = drive(body, size=(1280, 900), extra_args=[*LANG, "--force-dark-mode", "--enable-features=WebContentsForceDark"])
        reduced = drive(body, extra_args=[*LANG, "--force-prefers-reduced-motion"])
        self.assertEqual(light["panel"], "rgb(255, 255, 255)")
        self.assertEqual(dark["panel"], "rgb(31, 37, 44)")
        for result in (light, dark):
            for part in ("when", "activity", "sentBack"):
                self.assertGreaterEqual(result[part], 4.5, (part, result))
        self.assertEqual(reduced["mark"], "0s")


if __name__ == "__main__":
    unittest.main()
