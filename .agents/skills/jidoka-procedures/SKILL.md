---
name: jidoka-procedures
description: "Shared operating procedures for the jidoka-full, jidoka-balanced and jidoka-fast workflows: using the bundled skills, run reporting, continuing a run, workspace, tracker setup, model routing, finish, and pull request and CI. Read by those workflows; do not run it on its own."
disable-model-invocation: true
---

# Jidoka procedures

The `jidoka-full`, `jidoka-balanced` and `jidoka-fast` workflows read this file for the procedures they share. Each workflow keeps its own planning, testing, review and verification policy, and its instructions take precedence over this file where they differ. Do not start a run from this skill.

## Run ownership

The session that starts a run coordinates it to the end: the stages in order, the `jidoka` commands, run reporting, the interview and the hand-off of reviews. Individual tasks (a ticket, a review, a lookup, the pull request text) may go to agents as the workflow and model routing say; never the whole run or several stages to one agent. This holds even when other instructions say to delegate tasks.

## Using the bundled skills

In this project's checkout, the bundled skills are in `skills/<name>/`; in an installed bundle, they are siblings of this directory. Otherwise use the corresponding installed skills. `jidoka-grill-with-docs`, `jidoka-to-spec` and `jidoka-to-tickets` can only be invoked by the user, so the agent's skill tool may refuse them: read their `SKILL.md` from that location instead of invoking them. A workflow's instructions take precedence where they differ from those skills.

## Run reporting

Report progress to Jidoka dashboard with the `jidoka` CLI from the run's worktree (see Workspace). Update only at the points below; do not send periodic updates.

- **Start:** before Define, create the run. Normally `jidoka run begin` does this together with the workspace (see Workspace); keep the run ID it prints. When the workspace is not made that way, run `jidoka run start "<short task title>" --agent <agent>` and keep the printed run ID. `<agent>` is the coding agent you are running in: `claude` for Claude Code, `codex` for Codex, `copilot` for Copilot CLI. Jidoka uses it to pick that agent's model-routing policy, and says on standard error when it applies none; keep that for the Stage summaries. If the command stops with a message about the policy, tell the user what it says and stop. If the server is not running, start `jidoka dashboard` as a background process, then retry once. If a different or outdated server occupies the port, or the CLI rejects an option (including `--agent`), report that it needs to be stopped or upgraded.
- **Stage:** at the start of each later stage the workflow has, run `jidoka run update <id> <stage> running --activity "<what the stage is doing>"`.
- **Milestone:** after a meaningful milestone, such as tickets created or checks passing, run `jidoka run update <id> <stage> running --activity "<short phrase>"`. Keep the activity to one short line without secrets.
- **Ticket:** when work on a ticket with an HTTP(S) URL starts, add `--current-ticket "<url>"` to that update. When no ticket is in progress anymore, add `--clear-current-ticket`. Leave it unset for local Markdown tickets.
- **Run reports:** when a skill is used for the run outside a listed ticket, append `--run-report skill "<skill-name>"` to the next run update. At meaningful milestones outside a listed ticket, append `--run-report activity "<short outcome>"`. Repeat for multiple reports, including known delegated work. Report the workflow and procedures used at the first update after run creation, including skills loaded before it. Reports are retained and may be incomplete; never include secrets or transcripts.
- **Ticket reports:** once the ticket is listed, append `--ticket-report "<reference>" skill "<skill-name>"` when a skill is used for it and `--ticket-report "<reference>" activity "<short outcome>"` at meaningful milestones. Repeat the option for multiple reports. Use the ticket reference from the list, including local references, and the current stage; reports are retained and may be incomplete. Do not include secrets or transcripts. Include reports from delegated work when known.
- **Blocked:** when work cannot continue without outside input, run `jidoka run update <id> <stage> blocked --blocking-reason "<what is needed>"`. When work resumes, run `jidoka run update <id> <stage> running --clear-blocking-reason` (a Rework adds `--rework <n>` to that update). Waiting for the user's decision on Check findings is such a case; the Reworks section below names the reason.
- **Context:** record what a new session needs to continue the run, with `jidoka run update <id> <stage> <status>` options, only through the CLI and never by editing Jidoka's files. Each of these is a reference or a short summary, never a transcript, a secret or a token. Options left out keep their value; `--clear-<name>` removes one.
  - Once the worktree exists (`jidoka run begin` records these for you): `--mode <the workflow's name> --worktree <absolute path> --branch <branch> --base-commit <commit the worktree started from>`.
  - Once a spec or tickets exist: `--spec <url or absolute path>`, and one `--ticket "<reference> <title>"` per ticket in order (list them all each time; their statuses are kept). Record a local spec or ticket file by its absolute path, so the dashboard can open it. When a ticket starts, `--ticket-status <reference>=in-progress`; when it is finished, `--ticket-status <reference>=done`. A run with no tickets records none.
  - When a stage ends: `--summary <define|build|check|finish>=<one or two sentences>`. Summaries of earlier stages are kept.
  - When the pull request exists: `--pr <url>`.
  - A reason you block a run with `--blocking-reason` is the run's status reason; make it say what is needed.
- **Done:** once the pull request is open and its CI is green (or the project has no CI), run `jidoka run update <id> finish done --summary finish="<what shipped>" --activity "Opened <pull request URL>" --clear-current-ticket`.

Options that are left out keep their current value. If the server or an update fails, report the failure rather than claiming progress is tracked.

When the installed `jidoka` rejects a command or an option that these procedures use, such as `run begin`, `run check` or `pr sync`, the skills and the binary are out of step. Do not work around it by hand: report that Jidoka needs upgrading with `jidoka install` from a current release, mark the run blocked with that reason if the run exists, and stop.

## Continuing a run

When the user asks you to continue an existing run, or gives you a run ID, read its context with `jidoka run inspect <id>` (add `--json` for structured output) before changing anything, and check the recorded state against reality. Fields shown as unknown were never recorded: do not guess them.

Run `jidoka run check <id>` to compare the recorded state with the repository. It only reads. It reports each of the checks below as matching, differing with what it found, or unknown, and as not checked when it needs a worktree that is missing or differs. It exits with status 0 when nothing differs and 2 when something does. Every uncommitted change counts as a difference, so judge whether a listed change is expected. The checks are:

1. The recorded repository exists and shares its git directory with the worktree.
2. The recorded worktree exists and git lists it as a worktree.
3. That worktree is on the recorded branch.
4. The recorded base commit is an ancestor of `HEAD`.
5. The worktree has no unexpected changes, and any commits not yet pushed are listed.

Report every discrepancy and every unknown to the user, with what you found. Never reset, clean, check out over, delete or discard anything to make the state match, and do not start new work while something differs. If everything matches, read the recorded spec or saved plan from its reference (it holds the acceptance criteria), then continue in the recorded worktree from the recorded stage, at the first ticket that is not done, and record new context as you go. If something differs, ask the user how to proceed.

## Workspace

Each run must have its own fresh git worktree. Never switch branches or modify files in the user's main checkout, because the user may be working in it; the dedicated run worktree is where changes belong. Determine the project's documented PR base branch, otherwise the remote default from `git symbolic-ref refs/remotes/origin/HEAD`, and fetch it with `git fetch origin <base>`.

If the user explicitly starts this run in a fresh, dedicated worktree (for example, one provisioned by their agent environment), it may be the run's worktree. Verify that it is a linked worktree, is clean, has not been used for another run, and its `HEAD` equals `origin/<base>` after the fetch. Use a new feature branch there; if a branch is already checked out, only use it if it was created for this run and is not the base branch. Do not reset, clean, or discard changes to make a worktree eligible. In that case `jidoka run begin` is not used, because it makes its own worktree: run `jidoka run start`, then record the worktree, branch and base commit with `jidoka run update`, as described under Run reporting. Also make sure `.jidoka/` and `.scratch/` are listed in that worktree's `.gitignore`, adding the missing ones and committing the change with the first commit. If any condition is not met, create a new worktree instead, or stop and explain the constraint if the environment cannot do so.

Otherwise create the run and its workspace with `jidoka run begin "<short task title>" --agent <agent> --mode <the workflow's name> --base <base>`, where `<base>` is the project's documented PR base branch (without `--base` it uses the remote default). It creates the run, fetches the base, creates the worktree from `origin/<base>` on a new branch named from the title (`--branch <name>` chooses one), records mode, worktree, branch and base commit, and prints the run ID, worktree path, branch and base commit; `--json` prints them as data. Keep the printed run ID. If it ends with a blocked run and a retry command, fix what the reason says, or ask the user, and then run the retry command: it finishes what the failed attempt left and never deletes, resets or cleans anything. `begin` also adds `.jidoka/` and `.scratch/` to the worktree's `.gitignore` when they are missing and reports the added entries (`gitignore_added` in `--json`) or a warning; when it reports added entries, commit `.gitignore` with the first commit. If the worktree cannot be created, mark the run blocked with the reason and stop.

Run every later stage from the run's worktree: Define files, Build, checks, `jidoka run update` commands, commit, push, and the pull request. Respect the environment's permissions: a writable worktree does not necessarily grant write access to shared Git metadata or network access. Report any required permission rather than assuming it is available.

## Tracker setup

Follow this only when the workflow says the tracker is needed: always in jidoka-full, and in the lighter workflows only when the run reads or publishes a spec or tickets in the tracker. A clear, bounded request planned in the conversation does not need it, so never block such a run on it. When it is needed, check that `docs/agents/issue-tracker.md` exists in the run's worktree. If it is missing, look first in the checkout where the run started if it is distinct from the run worktree, then in the main checkout identified by `git worktree list --porcelain` (the first worktree, if it is not bare). If it exists in either checkout (setup was run but not committed to the base branch), copy that checkout's `docs/agents/` directory into the worktree and bring its `## Agent skills` section into the worktree's `AGENTS.md` or `CLAUDE.md` (whichever the source checkout edited), so the pull request includes the setup output; do not modify the source checkout. If it is absent from all available checkouts, or the main repository is bare and no setup source is available, mark the run blocked with `--blocking-reason "Run /jidoka-setup first"`, tell the user to run `/jidoka-setup` once in the original checkout and then start the run again (the new worktree will pick up its output; this run's worktree can be removed), and stop. Do not guess a tracker.

## Define rules

Do not require extra approval of test seams or a separate ticket-breakdown approval. Ask the user when a decision is actually needed.

## Saving the plan

When a run's plan and acceptance criteria exist only in the conversation, save them before Build so another session can continue without the original chat. Skip this when the spec is already a tracker issue or a file in the repository, and record that reference instead.

1. Take the plan path from `jidoka run begin` or `jidoka run inspect` (the `Plan` line, or `plan` in `--json`), and write a concise Markdown file with the task, the plan and the acceptance criteria there. It lives in the Jidoka data folder, outside the repository and the pull request, and stays after the worktree is cleaned up.
2. Record it with `jidoka run update <id> <stage> <status> --spec <absolute path>`. This is the same context option as a tracker spec, so no tracker is involved.
3. When the plan changes, rewrite the file. Never save secrets or a transcript.

## Verification

Find the project’s test and verification instructions in its agent instructions, contributing docs or build files. Follow its test scope and cadence; during iteration use targeted or quick checks appropriate to the change, unless the project or user requires more. Run all verification required before merging on the final candidate before the pull request. Reuse a passing result only while the checked code and relevant inputs are unchanged. A missing required tool or failing required check blocks the run: fix it or mark the run blocked, and never skip it. If no verification can be found, ask the user once, or say so in the pull request when running unattended.

## Committing for Check

Before Check, after the appropriate iteration checks, commit everything that belongs to the change, including new files but not scratch or generated files (ignore or remove those), with a normal commit (never amend), so `git status --porcelain` is empty. `jidoka-code-review` diffs committed work only, so uncommitted changes would be missed. Pin the review: the baseline is the base commit recorded for the run (`--base-commit`; read it with `jidoka run inspect`), and the reviewed commit is `HEAD`; record the reviewed commit's short hash with `--summary check="Review pinned at <hash>"` so a new session can find it, and repeat the latest reviewed hash in the Check summary you write when the stage ends, because a new summary replaces the old one.

## Reworks

When Check sends the run back to Build, report it as a Rework: `jidoka run update <id> build running --rework <n> --activity "Rework <n>: <findings being fixed>"`, numbering Reworks from 1 within the run and leaving the count set when Check runs again. A run gets at most three Reworks.

Fix actionable findings yourself within the Rework allowance; findings alone never block a run. Reworks 1 and 2 fix the actionable findings.

When findings remain after the second Rework, those that change only wording or documentation go into the third Rework without asking the user. Wording means text with no effect on what the program does or promises: comments, docs, docstrings, log messages, typos in internal names. A finding is behaviour when it changes what users or callers see or rely on: user-facing messages (especially when they name a different condition), public names and interfaces, outputs, or logic. If a remaining finding would change behaviour, the run needs a decision from the user, so report it as blocked: `jidoka run update <id> check blocked --blocking-reason "Decision needed on remaining Check findings"`. List those findings and ask the user whether to fix them in the third Rework too, finish with them listed in the pull request, or stop; the wording and documentation fixes still go into the third Rework unless they stop.

When findings remain after the third Rework, or a finding cannot be resolved, the run also needs a decision, so report it as blocked the same way. List the remaining findings and ask the user whether to do one more Rework, finish with the findings listed in the pull request, or stop. Follow their choice, and when work resumes clear the reason with `jidoka run update <id> <build|finish> running --clear-blocking-reason`. If they stop the run, leave it blocked.

## Changing agent skills

In Build, routed or not, when a ticket changes what an agent skill (a `SKILL.md` or the files it references) makes the agent do, decide, output or promise, use a skill-creator skill if the agent has one, and verify the change with realistic prompts run by a fresh agent that has the changed skill. Follow the project's own skill-authoring guidance where it has any. Record a `Skill check:` line in the commit message: what was run and its outcome, and when no skill-creator skill is available, say so there. Changes to wording only (typos, phrasing, formatting, links) need no check; the line says `wording only`.

## Model routing

How a run's model-routing selection applies to every Stage (Define, Build, Check and Finish) is in [references/model-routing.md](references/model-routing.md). Read it at the start of each Stage the workflow has, and again at the start of every Rework and every Check pass after one. The one exception is jidoka-fast: it never routes Define and Finish, so it reads this only at the start of Build and does not read it for Define or Finish.

## Finish

Use `jidoka-pr` to write the pull request description. When the user chose to finish with remaining Check findings, list them in the description. Commit anything still uncommitted that belongs to the change as a new commit. Verify the final commit before pushing: reuse the latest passing full verification when the final commit contains exactly the checked code and its relevant inputs are unchanged, otherwise run the project's verification on it. Report the result; do not open the pull request while a check fails or cannot run. Then follow [Pull request and CI](references/pull-request-and-ci.md) to open the pull request, and read it again while watching CI. The Finish report tells the user to run `jidoka run cleanup <id>` once the pull request merges.
