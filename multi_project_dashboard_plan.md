# Multi-project dashboard plan

The old dashboard served every repository from one page ([ADR 0033](https://github.com/TDSchreur/tds-factory/blob/HEAD/docs/adr/0033-one-dashboard-serves-every-registered-project.md)): a Project registry, a shared server on port 4300, and a row per Project. This plan carries that over to the new project, in the same small steps.

This plan builds on [dashboard_plan.md](dashboard_plan.md), step 5 (compact mode) and step 6 (the detail drawer) in particular, because Project rows reuse both.

## Where we are

The new project is already close to multi-project, and simpler than the old one:

- There is one server (`127.0.0.1:4311`) and one `runs.json` in the user data folder. It is not per repository.
- `factory run start` records the current directory as the Run's `project`, so Runs of many repositories already land in the same store.
- The page already shows each Run's project, but does not group by it.

The old design needed a registry, a state file and a port search because each repository had its own `.factory/` folder and its own server. None of that applies here. A Project is just the distinct `project` value of the Runs, so the first steps need no new backend.

## Decisions to make

- **Project identity.** Use the `project` path as stored, and show the folder name. When two Projects share a folder name, add the parent folder. Linked worktrees of one repository may report different paths. Decide in step 1 whether to fold them into the main checkout, which needs a git lookup at `run start`, or to accept the duplicates for now. Recommendation: accept duplicates first.
- **Empty and missing Projects.** With no registry, a Project without Runs does not exist. The old "No Runs" row and "missing folders" summary go away unless a registry is added in step 6. Recommendation: do not add one until it is needed.
- **Dashboard start.** `factory dashboard` today runs in the foreground in the first terminal. The old dashboard made later calls register and exit. A fixed port already gives one shared server, so only the "already running" case needs handling (step 5).

## Steps

Each step is one pull request, with a `changelog.d` fragment and tests. Each step names the workflow to run it with (`/factory-fast`, `/factory-balanced` or `/factory`); see the legend in [dashboard_plan.md](dashboard_plan.md).

1. **Project name and id.**
   - Run with `/factory-fast`: a small helper and a label change.
   - Prompt:

     ```text
     /factory-fast Step 1 of multi_project_dashboard_plan.md: Project name and id. Add a helper that derives a Project name from the project path (the folder name, with the parent folder added when two Projects share a name). Show the name on cards instead of the full path, with the path as a tooltip. Linked worktrees showing as separate Projects is accepted for now.
     ```
   - Add a small helper on the page to derive a Project name from the path, with the parent folder added on a clash.
   - Show the name on cards in place of the full path, with the path as a tooltip.
   - Decide the worktree question above.
2. **Group by Project.**
   - Run with `/factory-balanced`: restructures the board, so it needs tests and a review.
   - Prompt:

     ```text
     /factory-balanced Step 2 of multi_project_dashboard_plan.md: group by Project. Under the Stage headers show one row per Project, in alphabetical order, so Runs of different Projects never share a column. Keep the waiting-first order inside a row. No backend change.
     ```
   - Show one row per Project under the Stage headers, so Runs of different Projects never share a column.
   - Sort Project rows alphabetically, and keep the waiting-first order inside a row.
   - Needs no backend change.
3. **Collapse.**
   - Run with `/factory-fast`: a toggle with remembered state.
   - Prompt:

     ```text
     /factory-fast Step 3 of multi_project_dashboard_plan.md: collapse. Give each Project row a toggle button with aria-expanded that works with Enter and Space. A collapsed row is one line with the Runs per Stage and an "N waiting" badge. Remember the choice per Project in localStorage, tolerating blocked storage. A Run that starts waiting never expands a row by itself.
     ```
   - Give each Project row a toggle button with `aria-expanded`, working with Enter and Space.
   - A collapsed row is one line with the Runs per Stage and a "N waiting" badge.
   - Remember the choice per Project in `localStorage`. Blocked storage only means it is not remembered.
   - A Run that starts waiting never expands a row by itself.
4. **Global controls across Projects.**
   - Run with `/factory-balanced`: touches several earlier features at once.
   - Prompt:

     ```text
     /factory-balanced Step 4 of multi_project_dashboard_plan.md: global controls across Projects. Make "Needs you (N)", the compact Stage buttons and the notifications work over every Project row. Keep selection, expanded Done cards and focus per Project and Run id. Make Delete all in a Done column delete only that Project's Done Runs. Name the Project in Run detail and in notifications.
     ```
   - Make "Needs you (N)", the compact Stage buttons and the notifications work over every Project row.
   - Keep selection, expanded Done cards and focus per Project and Run id, so a Run id used in two Projects does not clash. Run ids are global in the store today, so this is a safeguard, not a need.
   - Make Delete all in the Done column delete only that Project's Done Runs.
   - Name the Project in Run detail and in notifications.
5. **One shared dashboard.**
   - Run with `/factory`: server lifecycle, a port probe and a new `/api/info` route.
   - Prompt:

     ```text
     /factory Step 5 of multi_project_dashboard_plan.md: one shared dashboard. When factory dashboard finds the server already running, print its URL and exit instead of failing to bind. Verify it through a new /api/info route (pid and Factory version), not the port alone, and warn but never stop an older version. Print the URL with ?project=<id> so that Project's row is first and expanded. Add a --port option for a separate dashboard that shares nothing.
     ```
   - When `factory dashboard` finds the server already running, print the URL and exit instead of failing to bind.
   - Probe `/api/info` (a pid and the Factory version) rather than trusting the port alone, and warn, never stop, when the running dashboard is an older version.
   - Add `?project=<id>` to the printed URL so that Project's row is first and expanded.
   - Offer a `--port` option for a separate dashboard that does not share anything.
6. **Optional: a Project registry.**
   - Run with `/factory`: new persisted state and a delete route.
   - Prompt:

     ```text
     /factory Step 6 of multi_project_dashboard_plan.md: Project registry. Only do this if empty Projects or missing folders matter. Add a projects.json next to runs.json, holding paths only; a Project joins on run start and on factory dashboard, and a failed write is a warning that never stops a Run. Add DELETE /api/projects/<id> and a summary row for Projects whose folders are gone, with Remove and Remove all missing, behind the existing host, origin and token checks.
     ```
   - Only if empty Projects, missing folders or the worktree fold-in turn out to matter.
   - A `projects.json` next to `runs.json`, holding paths only. A Project joins on `run start` or `factory dashboard`.
   - Add `DELETE /api/projects/<id>` and a summary row for Projects whose folders are gone, with Remove and Remove all missing.
   - Keep it best-effort: a failed registry write is a warning and never stops a Run.

## Not carried over

- Per-Project Run stores, PR providers and PR status caches. There is one store, and PR status is not planned yet.
- The `dashboard.json` state file and port 4300 fallback search, unless step 5 shows a fixed port is not enough.
