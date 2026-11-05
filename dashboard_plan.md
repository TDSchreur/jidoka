# Dashboard plan: carrying over the old dashboard

Source of the old dashboard: [TDSchreur/tds-factory](https://github.com/TDSchreur/tds-factory) (`dashboard/`, `docs/dashboard.md`, ADRs 0003, 0004, 0008, 0025, 0031).

Project rows and anything that spans several repositories are planned separately in [multi_project_dashboard_plan.md](multi_project_dashboard_plan.md).

## Where we are

The new dashboard is one file, `src/dashboard.html`, in plain JavaScript. It is embedded in the server and has no build step.

- It polls `/api/runs` and shows a vertical list of groups: Define, Build, Check, Finish and Done.
- Each card shows the title, project, status badge, Rework count, blocking reason and last-updated time.
- It has a System, Light and Dark theme switch, remembered in `localStorage`, and a build version in the footer.
- A Run has a title, project, stage, status, activity, current ticket, blocking reason, rework and `updated_at`.

## What the old dashboard had

The old dashboard was a Preact app of about 3,000 lines. It covered much more than the new backend can support, so it splits into three groups.

**Main functionality (carry over, in small steps):**

- A board with one column per Stage and a sticky header.
- Cards with a "needs you" marker and waiting Runs sorted first.
- A "Needs you (N)" filter and a count in the tab title.
- Compact mode below 1024 px: one Stage at a time, with stage buttons showing counts.
- A Run detail drawer (a native modal `<dialog>` on the right).
- Delete a Run.
- A welcome card when there are no Runs, and a Help panel.
- Browser notifications for "Needs you".

**Needs backend work that does not exist yet (later):**

- PR status, Check PR and Fix.
- Sync, Clean up, Accept Check and Grant extra loop.
- Split and Continue.
- Ticket tiles and token usage.
- Run metrics and the Fast/Full track.
- Stale-run detection from last-seen data.

## Decisions

- **Preact and htm, vendored, no build step** (the old ADR 0003). The files are committed as ES modules and loaded through an import map. The factory keeps no bundler and no runtime dependencies. Preact and htm come with their licenses, which go into the third-party notices.
- **Fluent 2 through design tokens** (the old ADR 0025). The stylesheet starts with a layer of tokens named as Fluent names them (`--colorNeutralBackground1`, `--fontSizeBase300`, `--shadow4`), taken from `@fluentui/tokens` `webLightTheme` and `webDarkTheme`. Every rule below the layer uses only tokens. The old `dashboard/style.css` has this layer and can be copied as a start.
- **Both are in place before any feature.** Every later step builds on components and tokens, so nothing gets written twice.

## Steps

Each step names the workflow to run it with. `/factory-fast` plans lightly and has no Check, `/factory-balanced` adds tests after the build and one review pass, and `/factory` is the full flow with Check and Rework. These are suggestions; switch up when a step turns out riskier than expected.

Each step is one pull request. It is small, shippable and independent of the later steps. Each one needs a `changelog.d` fragment, tests in `src/server.rs` for what the server serves, and a check against the accessibility guide.

0. **Preact base.**
   - Run with `/factory`: it is the foundation, and the file allow-list is a security surface.
   - Prompt:

     ```text
     /factory Step 0 of dashboard_plan.md: Preact base. Vendor preact, hooks and htm as ES modules in dashboard/vendor/ with their licenses (and add them to the third-party notices), loaded through an import map with no build step. Split src/dashboard.html into index.html, app.js and style.css. Have the server embed the files and serve them from an allow-list route with the right content types; an unknown file gives 404. Port the current page to components with no change in behavior: polling, groups, cards, theme switch and build footer. Keep the existing server tests passing and add tests for the allow-list and content types.
     ```
   - Vendor `preact`, `hooks` and `htm` into `dashboard/vendor/`, with their licenses.
   - Split the page into `index.html`, `app.js` and `style.css`, plus a `components.js` when it grows.
   - Have the server embed these files and serve them from an allow-list route, with the right content types. The build label stays injected into the page.
   - Port the current page to components with no change in behavior: polling, groups, cards, theme switch and footer.
   - Keep the existing server tests passing, and add tests for the allow-list (an unknown file gives 404) and for the content types.
1. **Fluent 2 design.**
   - Run with `/factory-balanced`: wide visual change, with a token test to write afterwards.
   - Prompt:

     ```text
     /factory-balanced Step 1 of dashboard_plan.md: Fluent 2 design. Add a layer of Fluent design tokens (webLightTheme and webDarkTheme from @fluentui/tokens, named as Fluent names them) at the top of the stylesheet; the old tds-factory dashboard/style.css has one to start from. Restyle the existing page with tokens only. Keep the System, Light and Dark theme switch. Add a test that no rule uses a literal color and that every token is used.
     ```
   - Add the token layer for light and dark, and restyle the existing page with tokens only.
   - Keep the theme switch (System, Light, Dark) and make dark use `webDarkTheme`.
   - Add a test that no rule uses a literal color and that every token is used.
2. **Board layout.**
   - Run with `/factory-balanced`: layout and accessibility need a review pass.
   - Prompt:

     ```text
     /factory-balanced Step 2 of dashboard_plan.md: board layout. Show Define, Build, Check, Finish and Done as columns on wide screens with a sticky header, and stack them on narrow screens. Keep the current cards. Follow the dashboard accessibility guide.
     ```
   - Show Define, Build, Check, Finish and Done as columns on wide screens, with a sticky header.
   - Stack them on narrow screens.
   - Keep the current cards.
3. **Card polish.**
   - Run with `/factory-fast`: small, contained changes to one component.
   - Prompt:

     ```text
     /factory-fast Step 3 of dashboard_plan.md: card polish. On blocked Runs add a "Needs you" label with the blocking reason. Show activity, a clickable ticket link (http and https only) and a "Rework n/2" chip. Show relative time ("Updated 5m ago") with the exact time as a tooltip. Sort blocked Runs first, then newest.
     ```
   - Add a "Needs you" label on blocked Runs, with the blocking reason.
   - Show activity, a clickable ticket link (http and https only) and a "Rework n/2" chip.
   - Show relative time ("Updated 5m ago"), with the exact time as a tooltip.
   - Sort blocked Runs first, then newest.
4. **Needs you.**
   - Run with `/factory-fast`: one filter and a title count.
   - Prompt:

     ```text
     /factory-fast Step 4 of dashboard_plan.md: Needs you. Add a "Needs you (N)" toggle in the header that filters the board to Runs waiting for the user, and put the count in the document title.
     ```
   - Add a "Needs you (N)" toggle in the header that filters the board.
   - Put the count in the document title.
5. **Compact mode (1024 px and below).**
   - Run with `/factory-balanced`: responsive behavior and focus handling are easy to get wrong.
   - Prompt:

     ```text
     /factory-balanced Step 5 of dashboard_plan.md: compact mode at 1024 px and below. Show one Stage at a time with Stage buttons that carry the number of visible Runs and a waiting marker. Open on the Stage that has a waiting Run, else the first non-empty Stage. Keep the selection across refreshes. Follow the dashboard accessibility guide.
     ```
   - Show one Stage at a time.
   - Add stage buttons with counts and a waiting marker.
   - Open on the Stage that has a waiting Run.
6. **Run detail drawer.**
   - Run with `/factory`: modal dialog, focus return and refresh behavior deserve the full check.
   - Prompt:

     ```text
     /factory Step 6 of dashboard_plan.md: Run detail drawer. Selecting a card opens a native modal dialog styled as a sheet on the right, over a full-width board with a light backdrop; full screen with a Back button at 1024 px and below. Show every field of the Run. Close with Escape, the backdrop or a button, and return focus to the card. Keep it open across refreshes and close it when the Run disappears. Follow the dashboard accessibility guide.
     ```
   - Open a native `<dialog>` on the right from a card.
   - Show every field of the Run.
   - Close with Escape, the backdrop or a button, and return focus to the card.
   - Keep it open across refreshes, and close it when the Run disappears.
7. **Done column and delete.**
   - Run with `/factory`: it adds a destructive write route, so the security check matters.
   - Prompt:

     ```text
     /factory Step 7 of dashboard_plan.md: Done column and delete. Collapse Done cards to one line with a Show details toggle. Add DELETE /api/runs/<id> with the existing host, origin and session token checks, and tests for each rejection. Add Delete per Run and Delete all in the Done column, each with a confirmation dialog. Delete only removes the Run record.
     ```
   - Collapse Done cards to one line, with a Show details toggle.
   - Add `DELETE /api/runs/<id>` with the existing host, origin and token checks.
   - Add Delete and Delete all, each with a confirmation.
   - Delete only removes the record. Worktree cleanup comes later.
8. **Welcome card and Help panel.**
   - Run with `/factory-fast`: mostly static content.
   - Prompt:

     ```text
     /factory-fast Step 8 of dashboard_plan.md: welcome card and Help panel. Show a welcome card instead of the columns when there are no Runs, saying how to start and linking to Help. Add a Help dialog in the header covering the Stages, "Needs you" and Rework, using the terms from GLOSSARY.md. Focus moves into the dialog and returns on close.
     ```
   - Show the welcome card when there are no Runs, with how to start.
   - Add a Help dialog covering the Stages, "Needs you" and Rework, using the glossary terms.
9. **Notifications.**
   - Run with `/factory-balanced`: permission flow and "already waiting" logic need tests.
   - Prompt:

     ```text
     /factory-balanced Step 9 of dashboard_plan.md: notifications. Add an opt-in browser notification, "Needs you in <Project>: <title>", when a Run starts waiting. Do not notify for Runs that were already waiting on first load. Handle denied and unsupported permission without errors.
     ```
   - Add an opt-in browser notification when a Run starts waiting: "Needs you in <Project>: <title>".
   - Do not notify for Runs that were already waiting on first load.

## Notes

- **Order.** Steps 0 and 1 are the base. Steps 2 to 5 give most of the old dashboard feel, so step 5 is a good moment to stop and review before the heavier steps.
- **Shipped skills.** Skills stay project-agnostic (see `AGENTS.md`). Nothing in this plan touches them.
