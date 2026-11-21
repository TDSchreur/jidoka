#!/usr/bin/env python3
"""Exercise the change groups section in Chromium; run with python3 tests/dashboard-groups.py.

The server side is faked: the page gets the group views a real server would build for the acceptance scenario
(see src/groups.rs), with GitHub and Azure DevOps pull request URLs.
The harness cannot send real key presses, so keyboard operation is checked structurally: the toggle is a native button
(which the browser activates with Enter and Space), it is in the tab order, takes focus and exposes aria-expanded.
"""
import json
import os
import unittest
import urllib.parse

from dashboard_browser import Handler, run_dashboard

AT = 1794081806000  # 2026-11-07 20:03:26 UTC
LANG = ["--lang=en-GB", "--accept-lang=en-GB"]
ENV = {**os.environ, "TZ": "Europe/Amsterdam", "LANGUAGE": "en_GB:en", "LC_ALL": "en_GB.UTF-8"}
GH = "https://github.com/acme/{}/pull/{}"
ADO = "https://dev.azure.com/acme/platform/_git/{}/pullrequest/{}"
EVIDENCE = "https://example.com/feed/module/2.4.0"


def run(id, title, repository, **extra):
    return {"id": id, "title": title, "project": repository, "repository": repository, "stage": "build",
            "status": "running", "updated_at": AT, "activity": "Working", **extra}


RUNS = [
    run("r-module", "Publish module", "/work/module", pr=GH.format("module", 1)),
    run("r-infra", "Use module in infra", "/work/infra", pr=GH.format("infra", 2)),
    run("r-api", "Use module in API", "/work/api", pr=GH.format("api", 3)),
    run("a-module", "Publish ADO module", "/work/ado-module", pr=ADO.format("module", 4)),
    run("a-infra", "Use ADO module", "/work/ado-infra", pr=ADO.format("infra", 5)),
    run("a-api", "Shipped ADO API", "/work/ado-api", status="done", stage="finish"),
]


def status(url, state):
    return {"url": url, "state": state, "checked_at": AT}


def dep(id, target, description, cleared, state, checked=False):
    out = {"id": id, "target": target, "description": description, "cleared": cleared, "status": state}
    if checked:
        out["checked_at"] = AT
    return out


def member(id, title, repository, pr, pr_status=None, dependencies=()):
    out = {"run": id, "missing": False, "title": title, "repository": repository, "dependencies": list(dependencies)}
    if pr:
        out["pr"] = pr
    if pr_status:
        out["pr_status"] = pr_status
    return out


# Acceptance scenario on GitHub: the module PR is merged, which clears only the PR dependency; publication is pending.
PR_DEP = {"pr": "r-module"}
MS_DEP = {"milestone": "m1"}
GITHUB_GROUP = {
    "id": "g1", "title": "Bicep module v2.4.0", "spec": "https://example.com/platform/changes/module-2-4",
    "summary": "1 of 3 attached PRs merged, 1 milestone pending",
    "members": [
        member("r-module", "Publish module", "/work/module", GH.format("module", 1), status(GH.format("module", 1), "merged")),
        member("r-infra", "Use module in infra", "/work/infra", GH.format("infra", 2), status(GH.format("infra", 2), "open"), [
            dep("d1", PR_DEP, "pull request of run r-module (/work/module)", True, "merged", checked=True),
            dep("d2", MS_DEP, "m1: Bicep module v2.4.0 published", False, "pending"),
        ]),
        member("r-api", "Use module in API", "/work/api", GH.format("api", 3), status(GH.format("api", 3), "draft"), [
            dep("d3", MS_DEP, "m1: Bicep module v2.4.0 published", False, "pending"),
        ]),
    ],
    "milestones": [{"id": "m1", "text": "Bicep module v2.4.0 published", "confirmed": False, "runs": ["r-infra", "r-api"]}],
}
# Azure DevOps: the milestone is confirmed with evidence, the module PR was replaced (no status for the new URL yet)
# and one member's run record is gone. The spec is a local file.
ADO_GROUP = {
    "id": "g2", "title": "ADO rollout", "spec": "/home/me/specs/rollout.md",
    "summary": "0 of 2 attached PRs merged, 0 milestones pending",
    "members": [
        member("a-module", "Publish ADO module", "/work/ado-module", ADO.format("module", 4)),
        member("a-infra", "Use ADO module", "/work/ado-infra", ADO.format("infra", 5), status(ADO.format("infra", 5), "closed"), [
            dep("d1", PR_DEP, "pull request of run a-module (/work/ado-module)", False, "unchecked"),
            dep("d2", MS_DEP, "m1: Module published", True, "confirmed"),
        ]),
        {"run": "a-gone", "missing": True, "dependencies": []},
    ],
    "milestones": [{"id": "m1", "text": "Module published", "confirmed": True, "confirmed_at": AT, "evidence": EVIDENCE, "runs": ["a-infra"]}],
}
EMPTY_GROUP = {"id": "g3", "title": "Nobody yet", "spec": "https://example.com/spec", "summary": "0 of 0 attached PRs merged", "members": []}

# Listed, but its own read fails (for example deleted since the list was read): the other groups still show.
UNREADABLE_GROUP = {"id": "g4", "title": "Gone", "spec": "https://example.com/spec", "members": [], "unreadable": True}

HELPERS = """
const wait = (ok, what) => new Promise((resolve, reject) => {
  const started = Date.now();
  const t = setInterval(() => { const v = ok(); if (v) { clearInterval(t); resolve(v); } else if (Date.now() - started > PAGE_WAIT_MS) { clearInterval(t); reject(new Error('timed out waiting for ' + what)); } }, 20);
});
const text = (el) => (el ? el.innerText.replace(/\\s+/g, ' ').trim() : null);
const post = (data) => fetch('/result', {method: 'POST', body: JSON.stringify(data)});
const run = (work) => work().then(post, (error) => post({error: String(error.stack || error)}));
"""


class GroupHandler(Handler):
    """Adds the group endpoints; `groups` is a list of views, or None for a server that has no /api/groups."""
    groups = None

    def do_GET(self):
        route = urllib.parse.urlsplit(self.path).path
        if route.startswith("/api/groups"):
            if self.groups is None:
                self.send_error(404)
                return
            if route == "/api/groups":
                body = [{"id": g["id"], "title": g["title"], "spec": g["spec"], "runs": [m["run"] for m in g["members"]]} for g in self.groups]
            else:
                body = next((g for g in self.groups if g["id"] == route.removeprefix("/api/groups/")), None)
                if body is None or body.get("unreadable"):
                    self.send_error(404)
                    return
            data = json.dumps(body).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(data)
            return
        super().do_GET()


def drive(body, groups):
    handler = type("Groups", (GroupHandler,), {"groups": groups})
    result = run_dashboard(json.loads(json.dumps(RUNS)), f"<script>{HELPERS}\nrun(async () => {{\n{body}\n}});</script>",
                           handler=handler, env=ENV, extra_args=LANG)
    if "error" in result:
        raise AssertionError(result["error"])
    return result


# The counts the board shows, read once the runs are listed.
COUNTS = """
await wait(() => document.querySelector('#line-title') && document.querySelector('#shipped-title'), 'the board');
const counts = {
  line: text(document.querySelector('#line-title')), shipped: text(document.querySelector('#shipped-title')),
  andon: text(document.querySelector('.andon-summary')), projects: text(document.querySelector('.projects-menu')),
  rows: document.querySelectorAll('[data-run-key]').length,
};
"""

SNAPSHOT = COUNTS + """
await wait(() => document.querySelector('#groups-title'), 'the groups section');
const toggles = [...document.querySelectorAll('.group-toggle')];
const rows = [...document.querySelectorAll('.group-row')].map((row) => text(row));
const collapsed = {expanded: toggles.map((t) => t.getAttribute('aria-expanded')), panels: document.querySelectorAll('.group-panel').length};
const first = toggles[0];
first.focus();
const keyboard = {
  native: first.tagName === 'BUTTON' && first.type === 'button', tabbable: first.tabIndex >= 0 && !first.disabled,
  focused: document.activeElement === first,
  controls: first.getAttribute('aria-controls'),
};
first.click();
const panel = await wait(() => document.querySelector('.group-panel'), 'the expanded group');
const expanded = {expanded: first.getAttribute('aria-expanded'), panelId: panel.id, text: text(panel), focusKept: document.activeElement === first};
const links = [...panel.querySelectorAll('a')].map((a) => a.getAttribute('href'));
first.click();
await wait(() => !document.querySelector('.group-panel'), 'the group to collapse');
const again = first.getAttribute('aria-expanded');
// A run in a group names it on its row and in its detail; one outside every group does not.
const chips = [...document.querySelectorAll('.line-row')].map((row) => [text(row.querySelector('.line-open')), [...row.querySelectorAll('.chip')].map(text).filter((t) => t.startsWith('Group:'))]);
document.querySelector('[data-run-key$="r-module"]').click();
const detail = await wait(() => text(document.querySelector('.detail-sub')), 'the run detail');
const section = text(document.querySelector('.detail-group'));
const sheetRuns = [...document.querySelectorAll('.detail-group-runs li')].map((li) => [text(li), Boolean(li.querySelector('button'))]);
// Another run of the group on this dashboard opens its own sheet; one that is not listed stays plain text.
[...document.querySelectorAll('.detail-group-runs button')].find((b) => text(b) === 'Use module in infra').click();
await wait(() => text(document.querySelector('#detail-title')) === 'Use module in infra', 'the other run sheet');
const switched = {open: document.querySelectorAll('dialog[open]').length, group: text(document.querySelector('.detail-group'))};
document.querySelector('.detail-close').click();
await wait(() => !document.querySelector('dialog[open]'), 'the detail to close');
// Group two, with the local spec, the replaced PR and the missing member.
toggles[1].click();
const second = await wait(() => document.querySelector('.group-panel'), 'the second group');
const specLink = document.querySelector('#groups-title').closest('section').querySelectorAll('.group-spec a');
return {counts, rows, collapsed, keyboard, expanded, links, again, second: text(second), secondLinks: [...second.querySelectorAll('a')].map((a) => a.getAttribute('href')),
  specLinks: [...specLink].map((a) => a.getAttribute('href')), chips, detail, section, sheetRuns, switched, heading: text(document.querySelector('#groups-title')),
  headings: [...document.querySelectorAll('.groups h2, .groups h3, .groups h4')].map((h) => h.tagName)};
"""


class GroupsTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.data = drive(SNAPSHOT, [GITHUB_GROUP, ADO_GROUP, EMPTY_GROUP, UNREADABLE_GROUP])

    def test_group_rows_show_title_summary_and_spec(self):
        data = self.data
        self.assertEqual(data["heading"], "Change groups (3)")
        self.assertIn("Bicep module v2.4.0 1 of 3 attached PRs merged, 1 milestone pending Spec: https://example.com/platform/changes/module-2-4", data["rows"][0])
        self.assertIn("ADO rollout 0 of 2 attached PRs merged, 0 milestones pending Spec: /home/me/specs/rollout.md", data["rows"][1])
        # A web spec is a link; a local path is plain text.
        self.assertEqual(data["specLinks"], ["https://example.com/platform/changes/module-2-4", "https://example.com/spec"])

    def test_runs_name_their_group_and_an_unreadable_group_is_left_out(self):
        chips = dict((title, groups) for title, groups in self.data["chips"])
        self.assertEqual(chips["Publish module"], ["Group: Bicep module v2.4.0"])
        self.assertEqual(chips["Use ADO module"], ["Group: ADO rollout"])
        self.assertEqual(chips["Shipped ADO API"] if "Shipped ADO API" in chips else [], [])
        self.assertNotIn("Group:", self.data["detail"])
        self.assertEqual(self.data["heading"], "Change groups (3)")

    def test_run_sheet_shows_its_group_and_opens_other_runs(self):
        data = self.data
        self.assertIn("Change group Bicep module v2.4.0", data["section"])
        self.assertIn("Spec: https://example.com/platform/changes/module-2-4", data["section"])
        self.assertEqual([runs for runs, _ in data["sheetRuns"]], [
            "Publish module (this run) module merged", "Use module in infra infra open", "Use module in API api a draft"])
        self.assertEqual([button for _, button in data["sheetRuns"]], [False, True, True])
        self.assertEqual(data["switched"]["open"], 1)
        self.assertIn("Use module in infra (this run)", data["switched"]["group"])
        self.assertIn("This run waits on", data["switched"]["group"])
        self.assertIn("Milestones Bicep module v2.4.0 published: pending", data["switched"]["group"])

    def test_keyboard_expand_and_collapse(self):
        data = self.data
        self.assertEqual(data["collapsed"], {"expanded": ["false", "false", "false"], "panels": 0})
        self.assertEqual(data["keyboard"], {"native": True, "tabbable": True, "focused": True, "controls": "group-panel-g1"})
        self.assertEqual(data["expanded"]["expanded"], "true")
        self.assertEqual(data["expanded"]["panelId"], "group-panel-g1")
        self.assertTrue(data["expanded"]["focusKept"])
        self.assertEqual(data["again"], "false")
        self.assertEqual(data["headings"][0], "H2")

    def test_github_merged_pr_clears_only_the_pr_dependency(self):
        text = self.data["expanded"]["text"]
        self.assertIn("Publish module module Pull request merged Last checked", text)
        self.assertIn("Waits on pull request of run r-module (/work/module): merged, cleared (last checked", text)
        self.assertEqual(text.count("cleared"), 1)  # the milestone dependencies stay pending
        self.assertEqual(text.count("Waits on m1: Bicep module v2.4.0 published: pending, pending"), 2)
        self.assertIn("Waits on m1: Bicep module v2.4.0 published: pending, pending", text)
        self.assertIn("Use module in infra", text)
        self.assertIn("open", text)
        self.assertIn("draft", text)
        self.assertIn("https://github.com/acme/module/pull/1", self.data["links"])

    def test_one_pending_milestone_lists_both_waiting_runs(self):
        text = self.data["expanded"]["text"]
        self.assertIn("Bicep module v2.4.0 published: pending. Waiting runs: Use module in infra, Use module in API.", text)
        self.assertNotIn("Evidence", text)

    def test_azure_replaced_pr_is_unchecked_and_pending_and_milestone_confirmed(self):
        text = self.data["second"]
        self.assertIn("Publish ADO module ado-module Pull request unchecked", text)
        self.assertNotIn("Last checked Publish", text)
        self.assertIn("Waits on pull request of run a-module (/work/ado-module): unchecked, pending", text)
        self.assertIn("Waits on m1: Module published: confirmed, cleared", text)
        self.assertIn("Module published: confirmed", text)
        self.assertIn("Evidence", text)
        self.assertIn("Waiting runs: Use ADO module.", text)
        self.assertIn("closed without merge", text)
        self.assertEqual(self.data["secondLinks"], ["https://dev.azure.com/acme/platform/_git/module/pullrequest/4",
                                                    "https://dev.azure.com/acme/platform/_git/infra/pullrequest/5", EVIDENCE])

    def test_missing_member_is_shown_as_run_missing(self):
        text = self.data["second"]
        self.assertIn("Run a-gone Pull request run missing", text)

    def test_runs_and_counts_are_unchanged_by_groups(self):
        without = drive(COUNTS + "return {counts};", None)["counts"]
        self.assertEqual(without["line"], "On the line (5)")
        self.assertEqual(without["shipped"], "Completed (1)")
        self.assertEqual(self.data["counts"], without)

    def test_missing_groups_endpoint_hides_the_section(self):
        result = drive(COUNTS + "await wait(() => performance.getEntriesByType('resource').some((e) => e.name.endsWith('/api/groups')), 'the groups request'); return {groups: document.querySelectorAll('.groups').length, message: text(document.querySelector('#message'))};", None)
        self.assertEqual(result, {"groups": 0, "message": ""})


if __name__ == "__main__":
    unittest.main()
