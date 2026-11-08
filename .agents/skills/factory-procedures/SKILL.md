---
name: factory-procedures
description: "Shared operating procedures for the factory-full, factory-balanced and factory-fast workflows: run reporting, continuing a run, workspace, tracker setup, model routing, and pull request and CI. Read by those workflows; do not run it on its own."
disable-model-invocation: true
---

# Factory procedures

The `factory-full`, `factory-balanced` and `factory-fast` workflows read this file for the procedures they share. Each workflow keeps its own planning, testing, review and verification policy, and its instructions take precedence over this file where they differ. Do not start a run from this skill.

## Run reporting

Report progress to the factory dashboard with the `factory` CLI from the run's worktree (see Workspace). Update only at the points below; do not send periodic updates.

- **Start:** before Define, create the run. Normally `factory run begin` does this together with the workspace (see Workspace); keep the run ID it prints. When the workspace is not made that way, run `factory run start "<short task title>" --agent <agent>` and keep the printed run ID. `<agent>` is the coding agent you are running in: `claude` for Claude Code, `codex` for Codex, `copilot` for Copilot CLI. The factory uses it to pick that agent's model-routing policy, and says on standard error when it applies none; keep that for the Stage summaries. If the command stops with a message about the policy, tell the user what it says and stop. If the server is not running, start `factory dashboard` as a background process, then retry once. If a different or outdated server occupies the port, or the CLI rejects an option (including `--agent`), report that it needs to be stopped or upgraded.
- **Stage:** at the start of each later stage the workflow has, run `factory run update <id> <stage> running --activity "<what the stage is doing>"`.
- **Milestone:** after a meaningful milestone, such as tickets created or checks passing, run `factory run update <id> <stage> running --activity "<short phrase>"`. Keep the activity to one short line without secrets.
- **Ticket:** when work on a ticket with an HTTP(S) URL starts, add `--current-ticket "<url>"` to that update. When no ticket is in progress anymore, add `--clear-current-ticket`. Leave it unset for local Markdown tickets.
- **Run reports:** when a skill is used for the run outside a listed ticket, append `--run-report skill "<skill-name>"` to the next run update. At meaningful milestones outside a listed ticket, append `--run-report activity "<short outcome>"`. Repeat for multiple reports, including known delegated work. Report the workflow and procedures used at the first update after run creation, including skills loaded before it. Reports are retained and may be incomplete; never include secrets or transcripts.
- **Ticket reports:** once the ticket is listed, append `--ticket-report "<reference>" skill "<skill-name>"` when a skill is used for it and `--ticket-report "<reference>" activity "<short outcome>"` at meaningful milestones. Repeat the option for multiple reports. Use the ticket reference from the list, including local references, and the current stage; reports are retained and may be incomplete. Do not include secrets or transcripts. Include reports from delegated work when known.
- **Blocked:** when work cannot continue without outside input, run `factory run update <id> <stage> blocked --blocking-reason "<what is needed>"`. When work resumes, run `factory run update <id> <stage> running --clear-blocking-reason` (a Rework adds `--rework <n>` to that update). Waiting for the user's decision on Check findings is such a case; the workflows with Check name the reason.
- **Context:** record what a new session needs to continue the run, with `factory run update <id> <stage> <status>` options, only through the CLI and never by editing the factory's files. Each of these is a reference or a short summary, never a transcript, a secret or a token. Options left out keep their value; `--clear-<name>` removes one.
  - Once the worktree exists (`factory run begin` records these for you): `--mode <the workflow's name> --worktree <absolute path> --branch <branch> --base-commit <commit the worktree started from>`.
  - Once a spec or tickets exist: `--spec <url or path>`, and one `--ticket "<reference> <title>"` per ticket in order (list them all each time; their statuses are kept). When a ticket starts, `--ticket-status <reference>=in-progress`; when it is finished, `--ticket-status <reference>=done`. A run with no tickets records none.
  - When a stage ends: `--summary <define|build|check|finish>=<one or two sentences>`. Summaries of earlier stages are kept.
  - When the pull request exists: `--pr <url>`.
  - A reason you block a run with `--blocking-reason` is the run's status reason; make it say what is needed.
- **Done:** once the pull request is open and its CI is green (or the project has no CI), run `factory run update <id> finish done --summary finish="<what shipped>" --activity "Opened <pull request URL>" --clear-current-ticket`.

Options that are left out keep their current value. If the server or an update fails, report the failure rather than claiming progress is tracked.

When the installed `factory` rejects a command or an option that these procedures use, such as `run begin`, `run check` or `pr sync`, the skills and the binary are out of step. Do not work around it by hand: report that the factory needs upgrading with `factory install` from a current release, mark the run blocked with that reason if the run exists, and stop.

## Continuing a run

When the user asks you to continue an existing run, or gives you a run ID, read its context with `factory run inspect <id>` (add `--json` for structured output) before changing anything, and check the recorded state against reality. Fields shown as unknown were never recorded: do not guess them.

Run `factory run check <id>` to compare the recorded state with the repository. It only reads. It reports each of the checks below as matching, differing with what it found, or unknown, and as not checked when it needs a worktree that is missing or differs. It exits with status 0 when nothing differs and 2 when something does. Every uncommitted change counts as a difference, so judge whether a listed change is expected. The checks are:

1. The recorded repository exists and shares its git directory with the worktree.
2. The recorded worktree exists and git lists it as a worktree.
3. That worktree is on the recorded branch.
4. The recorded base commit is an ancestor of `HEAD`.
5. The worktree has no unexpected changes, and any commits not yet pushed are listed.

Report every discrepancy and every unknown to the user, with what you found. Never reset, clean, check out over, delete or discard anything to make the state match, and do not start new work while something differs. If everything matches, read the recorded spec or saved plan from its reference (it holds the acceptance criteria), then continue in the recorded worktree from the recorded stage, at the first ticket that is not done, and record new context as you go. If something differs, ask the user how to proceed.

## Workspace

Each run must have its own fresh git worktree. Never switch branches or modify files in the user's main checkout, because the user may be working in it; the dedicated run worktree is where changes belong. Determine the project's documented PR base branch, otherwise the remote default from `git symbolic-ref refs/remotes/origin/HEAD`, and fetch it with `git fetch origin <base>`.

If the user explicitly starts this run in a fresh, dedicated worktree (for example, one provisioned by their agent environment), it may be the run's worktree. Verify that it is a linked worktree, is clean, has not been used for another run, and its `HEAD` equals `origin/<base>` after the fetch. Use a new feature branch there; if a branch is already checked out, only use it if it was created for this run and is not the base branch. Do not reset, clean, or discard changes to make a worktree eligible. In that case `factory run begin` is not used, because it makes its own worktree: run `factory run start`, then record the worktree, branch and base commit with `factory run update`, as described under Run reporting. Also make sure `.factory/` and `.scratch/` are listed in that worktree's `.gitignore`, adding the missing ones and committing the change with the first commit. If any condition is not met, create a new worktree instead, or stop and explain the constraint if the environment cannot do so.

Otherwise create the run and its workspace with `factory run begin "<short task title>" --agent <agent> --mode <the workflow's name> --base <base>`, where `<base>` is the project's documented PR base branch (without `--base` it uses the remote default). It creates the run, fetches the base, creates the worktree from `origin/<base>` on a new branch named from the title (`--branch <name>` chooses one), records mode, worktree, branch and base commit, and prints the run ID, worktree path, branch and base commit; `--json` prints them as data. Keep the printed run ID. If it ends with a blocked run and a retry command, fix what the reason says, or ask the user, and then run the retry command: it finishes what the failed attempt left and never deletes, resets or cleans anything. `begin` also adds `.factory/` and `.scratch/` to the worktree's `.gitignore` when they are missing and reports the added entries (`gitignore_added` in `--json`) or a warning; when it reports added entries, commit `.gitignore` with the first commit. If the worktree cannot be created, mark the run blocked with the reason and stop.

Run every later stage from the run's worktree: Define files, Build, checks, `factory run update` commands, commit, push, and the pull request. Respect the environment's permissions: a writable worktree does not necessarily grant write access to shared Git metadata or network access. Report any required permission rather than assuming it is available.

## Tracker setup

Follow this only when the workflow says the tracker is needed: always in `/factory-full`, and in the lighter workflows only when the run reads or publishes a spec or tickets in the tracker. A clear, bounded request planned in the conversation does not need it, so never block such a run on it. When it is needed, check that `docs/agents/issue-tracker.md` exists in the run's worktree. If it is missing, look first in the checkout where the run started if it is distinct from the run worktree, then in the main checkout identified by `git worktree list --porcelain` (the first worktree, if it is not bare). If it exists in either checkout (setup was run but not committed to the base branch), copy that checkout's `docs/agents/` directory into the worktree and bring its `## Agent skills` section into the worktree's `AGENTS.md` or `CLAUDE.md` (whichever the source checkout edited), so the pull request includes the setup output; do not modify the source checkout. If it is absent from all available checkouts, or the main repository is bare and no setup source is available, mark the run blocked with `--blocking-reason "Run /factory-setup first"`, tell the user to run `/factory-setup` once in the original checkout and then start the run again (the new worktree will pick up its output; this run's worktree can be removed), and stop. Do not guess a tracker.

## Saving the plan

When a run's plan and acceptance criteria exist only in the conversation, save them before Build so another session can continue without the original chat. Skip this when the spec is already a tracker issue or a file in the repository, and record that reference instead.

1. Write a concise Markdown file with the task, the plan and the acceptance criteria to `factory-plan.md` in the run worktree's git directory (`git rev-parse --absolute-git-dir`). If that is not writable, write `<worktree path>.factory-plan.md` beside the worktree instead and remove it together with the worktree. The file stays out of the pull request.
2. Record it with `factory run update <id> <stage> <status> --spec <absolute path>`. This is the same context option as a tracker spec, so no tracker is involved.
3. When the plan changes, rewrite the file. Never save secrets or a transcript.

## Verification

Find the project's verification in its agent instructions, contributing docs or build files: the commands to run before merging, which should match CI. Run all of it at the cadence set by the workflow. Reuse a passing result only while the checked code and relevant inputs are unchanged, and honor stricter project requirements. A missing tool or failing check blocks the run: fix it or mark the run blocked, and never skip it. If no verification can be found, ask the user once, or say so in the pull request when running unattended.

## Committing for Check

Before Check, commit the completed implementation: make sure the full verification has passed on exactly this code (rerun it if anything changed since it ran), then commit everything that belongs to the change, including new files but not scratch or generated files (ignore or remove those), with a normal commit (never amend), so `git status --porcelain` is empty. `factory-code-review` diffs committed work only, so uncommitted changes would be missed. Pin the review: the baseline is the base commit recorded for the run (`--base-commit`; read it with `factory run inspect`), and the reviewed commit is `HEAD`; record the reviewed commit's short hash with `--summary check="Review pinned at <hash>"` so a new session can find it, and repeat the latest reviewed hash in the Check summary you write when the stage ends, because a new summary replaces the old one.

## Reworks

When Check sends the run back to Build, report it as a Rework: `factory run update <id> build running --rework <n> --activity "Rework <n>: <findings being fixed>"`, numbering Reworks from 1 within the run and leaving the count set when Check runs again. A run gets at most three Reworks.

Fix actionable findings yourself within the Rework allowance; findings alone never block a run. Reworks 1 and 2 fix the actionable findings.

When findings remain after the second Rework, those that change only wording or documentation go into the third Rework without asking the user. If a remaining finding would change behaviour, the run needs a decision from the user, so report it as blocked: `factory run update <id> check blocked --blocking-reason "Decision needed on remaining Check findings"`. List those findings and ask the user whether to fix them in the third Rework too, finish with them listed in the pull request, or stop; the wording and documentation fixes still go into the third Rework unless they stop.

When findings remain after the third Rework, or a finding cannot be resolved, the run also needs a decision, so report it as blocked the same way. List the remaining findings and ask the user whether to do one more Rework, finish with the findings listed in the pull request, or stop. Follow their choice, and when work resumes clear the reason with `factory run update <id> <build|finish> running --clear-blocking-reason`. If they stop the run, leave it blocked.

## Model routing

How a run's model-routing selection applies to every Stage (Define, Build, Check and Finish) is in [references/model-routing.md](references/model-routing.md). All three workflows read it at the start of each Stage they have, and again at every Rework and Check pass after one.

## Pull request and CI

The factory's pull request commands work with two hosts, and each needs its own command line tool, signed in: `gh` for GitHub, and `az` with the `azure-devops` extension for Azure Repos (signed in with `az login`, or with a personal access token in `AZURE_DEVOPS_EXT_PAT`). They work out the host from `origin`. `factory doctor` shows the host it found, and whether the tool and a sign-in are there. For any other host, or when the host is not configured, open the pull request and watch its CI with that host's own tooling, then record the URL on the run with `--pr <url>`. `factory pr sync` needs only git, so it works on any host; the rest of this section still applies, with the host's own tooling in place of `factory pr open` and `factory pr wait`. Watch every check there until all have completed, noticing failures as well as success, with `--activity "Waiting for CI"` on the running run.

### Opening the pull request

Once the project's verification has passed on the final commit, push the branch and create a pull request targeting the base branch chosen in Workspace. Write the title and the description first (the description in a file), then run `factory pr open <run-id> --title "<title>" --body-file <path> --base <base>`. It checks that the run's worktree is clean and on the recorded branch, that the branch is not the base and has commits beyond it, and that the host's tool is signed in; then it pushes the branch (never forced), opens the pull request, or finds the one that is already open for the branch, records its URL on the run, and prints it (`--json` prints data). If it stops with a message, fix what it says and run it again: a retry never makes a second pull request.

### Watching CI

Watch the pull request's CI with `factory pr wait <run-id>`. It follows the checks of the pull request's current head commit until all of them have ended, sets the activity of a running run to "Waiting for CI" itself, and prints one outcome and a matching exit status:

- `passed` (0): every check passed or was skipped.
- `failed` (2): a check failed. The output lists each failed check with its link, and the end of the failed log of up to three runs.
- `cancelled` (3): checks were cancelled and none failed.
- `timeout` (4): checks were still running at the end of `--timeout` (default 30 minutes).
- `head-changed` (5): the pull request got a new head commit while waiting, so the results say nothing about it; run the command again.
- `no-ci` (6): no check appeared within 90 seconds (or within `--timeout` when that is shorter), so the project has no CI.
- `not-open` (7): the pull request was merged or closed.

Exit status 1 is a problem with the tools or the run (no `gh` or `az`, not signed in, no network, no pull request recorded); its message says what to do. Next to the outcome the command says whether the branch is up to date, behind its base or in conflict with it. It only reads: it never merges, pushes or updates the branch, and a `passed` outcome does not mark Finish done. Add `--fail-fast` to get a `failed` outcome as soon as one check fails.

### Keeping the branch current

Keep the pull request current with its base branch. A base branch that requires up-to-date branches refuses a stale pull request even when CI is green, and another pull request may land while yours is open. Check before you push, and again each time a CI watch ends or a check fails. Run `factory pr sync <run-id> --base <base>`. It fetches the base, checks that the worktree is clean and on the recorded branch, and reports one outcome:

- `current` (0): the branch already contains the base.
- `pushed` (2): it merged the base with a normal merge commit, the base touched none of the change's own files, and it pushed the merge; watch CI again with `factory pr wait`.
- `needs-verification` (3): it merged the base cleanly, but the base touched files of the change, which it lists; rerun the project's verification on the merged code, then push.
- `conflict` (4): the merge is left in progress with the conflicting files listed; resolve them, rerun the verification on the merged code, commit and push.

Exit status 1 is a problem it explains, such as a dirty tree or a failed fetch. It never rebases, amends, force-pushes, resets or aborts, it never pushes commits of yours that are not pushed yet (it reports them and does not push the merge either), a renamed file counts as touched, and if its own push of a clean merge failed, running it again pushes that merge; it never pushes a merge that needs verification or a conflict you resolved. A base update is neither a Rework nor a CI fix; `factory pr sync` notes it on the run. A conflict you cannot resolve with confidence blocks the run with that reason.

Once the pull request exists, the base state that `factory pr wait` prints is a second view. On GitHub, `gh pr view --json mergeStateStatus` is a third: `BEHIND` means behind, `DIRTY` means it already conflicts with the base, and `UNKNOWN` means GitHub is still working it out, so ask again until it says something else rather than reading it as current. On Azure Repos, rely on `factory pr sync` for the base state.

Whenever you add to the branch, whether to resolve a conflict or to fix CI, make a new commit and never rebase, amend or force-push. The review is pinned to a commit hash, and rewritten history would leave that pin, and the reviewers' comments, pointing at commits that no longer exist.

### When CI fails

When the outcome is `failed`, read the failure details it printed (fetch the full log only when the excerpt is not enough), fix the cause in a new commit on the same branch (never amend or force-push, as above), rerun the project's verification on the changed code, push, note it with a milestone update, and run `factory pr wait` again; repeat until CI is green. CI fixes are not Reworks.

- After `head-changed`, run it again.
- For `cancelled` or `timeout`, look at the checks it lists; if the cause is not something you can fix, treat it as a failure you cannot fix.
- A failure you cannot fix, such as an infrastructure problem, a missing permission, or a flaky external dependency, blocks the run with that reason.
- On `no-ci`, skip the watch and say so.
- On `not-open`, report that the pull request was merged or closed and stop.

### Reporting the result

Never merge the pull request: merging is the human's review decision, and the run ends at a pull request that is ready for review. Report the pull request as ready for review only when the outcome is `passed` and the pull request is not behind its base. Keep the worktree for review follow-ups. Report its link, the worktree path (removable with `git worktree remove <path>` after merge), the checks performed, and the final CI result. If a required tool, permission, or repository configuration is unavailable, report what remains, mark the run blocked with that as the reason, and stop.
