#!/usr/bin/env python3
"""Exercise runHistory (dashboard/history.js) with run JSON in Chromium; run with python3 tests/dashboard-run-history.py."""
import json
import unittest

from dashboard_browser import run_dashboard

NOW = 10_000
T1 = "local-1"
T2 = "local-2"
TICKETS = [{"reference": T1, "title": "First", "status": "done"}, {"reference": T2, "title": "Second", "status": "pending"}]


def mark(at, stage, **extra):
    return {"at": at, "stage": stage, **extra}


def report(text, at, **extra):
    return {"kind": "activity", "text": text, "at": at, **extra}


def finding(id, state, at, **extra):
    return {"id": id, "check": "review", "text": f"Problem {id}", "state": state, "at": at, **extra}


BASE = {"id": "r", "title": "Run", "status": "running", "stage": "check", "updated_at": NOW, "tickets": TICKETS}

FIXTURES = {
    "timeline": {
        **BASE, "stage": "build",
        "timeline": [mark(1000, "define"), mark(2000, "build", ticket=T1, activity="Starting"), mark(3000, "build", ticket=T2), mark(4000, "check")],
        "summaries": {"define": "Scoped", "build": "Built", "finish": "Orphan"},
        "run_reports": [report("scoped", 1500, stage="define"), report("built", 2500, stage="build")],
        "ticket_reports": [report("t1 work", 2100, stage="build", ticket=T1)],
    },
    "station": {
        **BASE, "stage": "build", "rework": 1,
        "summaries": {"build": "Built"},
        "run_reports": [report("built", 1, stage="build"), report("stray", 2, stage="elsewhere")],
        "ticket_reports": [report("t1 work", 1, stage="build", ticket=T1), report("unlisted", 1, ticket="local-9")],
    },
    "rework": {
        **BASE, "rework": 1,
        "timeline": [mark(1000, "build", ticket=T1), mark(2000, "check"), mark(3000, "build", rework=1), mark(4000, "check", rework=1)],
        "findings": [
            finding("F1", "fixed", 2500, rework=1, reviewed_commit="abcdef123456", fix_commit="1234567890"),
            finding("F2", "deferred", 2600, resolved_at=4500),
            {"id": "bad", "state": "open"},
        ],
    },
    "events": {
        **BASE, "status": "blocked",
        "timeline": [mark(1000, "build", ticket=T1), mark(5000, "check")],
        "events": [{"kind": "blocked", "at": 2000, "reason": "Waiting"}, {"kind": "resumed", "at": 3000}, {"kind": "blocked", "at": 6000}],
        "usage": {"pauses": [{"at": 1500, "kind": "usage_limit", "resumed_at": 1800}], "idle": [{"from": 4000, "to": 4500}, {"from": 4600, "to": 4600}]},
    },
    "window": {
        **BASE,
        "timeline": [mark(1000, "build", ticket=T1), mark(2000, "check")],
        "run_reports": [report("no stage, in build", 1500), report("no stage, in check", 2500), report("no stage, before all", 500)],
    },
    "unlisted": {
        **BASE,
        "timeline": [mark(1000, "build", ticket=T1), mark(2000, "check")],
        "ticket_reports": [report("not listed", 1500, stage="build", ticket="local-9")],
    },
    "unstarted": {
        **BASE,
        "timeline": [mark(1000, "define"), mark(2000, "check")],
        "ticket_reports": [report("waiting", 2500, ticket=T2)],
    },
}

# Reports the shape of every stop and keeps only what the assertions need.
BODY = f"""
const {{runHistory}} = await import('/assets/history.js');
const fixtures = {json.dumps(FIXTURES)};
const texts = (reports) => reports.map((r) => r.text);
const out = {{}};
for (const [name, run] of Object.entries(fixtures)) {{
  const history = runHistory(run, {NOW});
  out[name] = {{
    keys: history.stops.map((stop) => Object.keys(stop).sort().join()),
    stops: history.stops.map((stop) => ({{
      stage: stop.stage, name: stop.name, kind: stop.kind, rework: stop.rework, start: stop.start, duration: stop.duration, state: stop.state,
      activities: stop.activities, summary: stop.summary, sentBack: stop.sentBack, progress: stop.progress,
      reports: texts(stop.reports), events: stop.events.map((e) => e.kind),
      tickets: stop.tickets.map((t) => [t.ticket.reference, texts(t.reports)]),
      heading: stop.findings.heading, lists: stop.findings.lists.map((l) => [l.label, l.findings.map((f) => f.id), l.outcome]),
    }})),
    other: {{...history.other, reports: texts(history.other.reports), events: history.other.events.map((e) => e.kind), tickets: history.other.tickets.map((t) => [t.ticket.reference, texts(t.reports)])}},
  }};
}}
return out;
"""


class RunHistoryTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        script = f"<script>const run = (work) => work().then((d) => fetch('/result', {{method: 'POST', body: JSON.stringify(d)}}), (e) => fetch('/result', {{method: 'POST', body: JSON.stringify({{error: String(e.stack || e)}})}}));\nrun(async () => {{\n{BODY}\n}});</script>"
        result = run_dashboard([], script)
        if "error" in result:
            raise AssertionError(result["error"])
        cls.result = result

    def stops(self, name, key="stage"):
        return [stop[key] for stop in self.result[name]["stops"]]

    def test_timeline_gives_one_pass_per_run_of_build_marks(self):
        stops = self.result["timeline"]["stops"]
        self.assertEqual(self.stops("timeline"), ["define", "build", "check"])
        self.assertEqual({stop["kind"] for stop in stops}, {"pass"})
        build = stops[1]
        self.assertEqual((build["start"], build["duration"], build["state"]), (2000, 2000, "done"))
        self.assertEqual(build["activities"], ["Starting"])
        self.assertEqual(build["summary"], "Built")
        self.assertEqual(build["progress"], {"done": 1, "total": 2})
        self.assertEqual(build["reports"], ["built"])
        self.assertEqual(build["tickets"], [[T1, ["t1 work"]], [T2, []]])
        self.assertEqual(stops[0]["reports"], ["scoped"])
        self.assertEqual((stops[2]["state"], stops[2]["duration"]), ("current", NOW - 4000))
        self.assertEqual(self.result["timeline"]["other"]["summaries"], [["Finish", "Orphan"]])

    def test_every_stop_has_the_same_shape(self):
        keys = {key for name in ("timeline", "station") for key in self.result[name]["keys"]}
        self.assertEqual(len(keys), 1)

    def test_run_without_a_timeline_has_a_station_per_stage(self):
        history = self.result["station"]
        self.assertEqual(self.stops("station"), ["define", "build", "check", "finish"])
        self.assertEqual(self.stops("station", "kind"), ["station"] * 4)
        self.assertEqual(self.stops("station", "state"), ["done", "current", "ahead", "ahead"])
        build = history["stops"][1]
        self.assertEqual(build["reports"], ["built"])
        self.assertEqual(build["summary"], "Built")
        self.assertEqual(build["tickets"], [[T1, ["t1 work"]], [T2, []]])
        self.assertEqual(history["stops"][2]["rework"], 1)
        self.assertEqual(history["other"]["reports"], ["stray", "unlisted"])
        self.assertEqual(history["other"]["tickets"], [])

    def test_rework_places_findings_on_the_check_and_build_passes(self):
        stops = self.result["rework"]["stops"]
        self.assertEqual([(s["stage"], s["rework"]) for s in stops], [("build", None), ("check", None), ("build", 1), ("check", 1)])
        sent_back = stops[1]
        self.assertTrue(sent_back["sentBack"])
        self.assertEqual(sent_back["heading"], "Sent back with 1 finding · reviewed abcdef1")
        self.assertEqual(sent_back["lists"], [["Findings that sent Check back", ["F1", "F2"], False]])
        self.assertEqual(stops[2]["lists"], [["Findings for Rework 1", ["F1"], True]])
        last = stops[3]
        self.assertEqual(last["heading"], "0 findings")
        self.assertEqual(last["lists"], [["Findings resolved in this Check", ["F2"], True]])

    def test_events_pauses_and_idle_gaps_go_on_the_pass_that_holds_them(self):
        stops = self.result["events"]["stops"]
        self.assertEqual(stops[0]["events"], ["pause", "blocked", "resumed", "idle"])
        self.assertEqual(stops[1]["events"], ["blocked"])
        self.assertEqual(self.result["events"]["other"]["events"], [])
        self.assertEqual(stops[1]["state"], "blocked")

    def test_report_without_a_stage_goes_by_time_window(self):
        stops = self.result["window"]["stops"]
        self.assertEqual(stops[0]["reports"], ["no stage, in build"])
        self.assertEqual(stops[1]["reports"], ["no stage, in check"])
        self.assertEqual(self.result["window"]["other"]["reports"], ["no stage, before all"])

    def test_ticket_report_for_an_unlisted_ticket_goes_to_other(self):
        self.assertEqual(self.result["unlisted"]["other"]["reports"], ["not listed"])
        self.assertEqual([stop["reports"] for stop in self.result["unlisted"]["stops"]], [[], []])

    def test_tickets_no_pass_started_are_listed_apart(self):
        other = self.result["unstarted"]["other"]
        self.assertEqual(other["tickets"], [[T1, []], [T2, ["waiting"]]])
        self.assertEqual(other["reports"], [])
        self.assertTrue(all(not stop["tickets"] for stop in self.result["unstarted"]["stops"]))


if __name__ == "__main__":
    unittest.main()
