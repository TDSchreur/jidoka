---
name: factory
description: Take a software change from an idea to a pull request through define, build, check, and finish. Use when the user asks to run factory.
---

# TDS Factory

Take the user's request through these stages in order. Tell the user what each stage produced. Use the named skills, reading their `SKILL.md` files and any referenced resources when needed. In this project's checkout, the bundled upstream skills are in `skills/<name>/`; in an installed bundle, they are siblings of this directory. Otherwise use the corresponding installed skills. This workflow's instructions take precedence where they differ from those skills.

## Run reporting

Report progress to the factory dashboard with the `factory` CLI from the run's worktree (see Workspace). Update only at the points below; do not send periodic updates.

- **Start:** before Define, run `factory run start "<short task title>"` and keep the printed run ID. If the server is not running, start `factory dashboard` as a background process, then retry once. If a different or outdated server occupies the port, or the CLI rejects an option below, report that it needs to be stopped or upgraded.
- **Stage:** at the start of each later stage, run `factory run update <id> <build|check|finish> running --activity "<what the stage is doing>"`.
- **Milestone:** after a meaningful milestone, such as tickets created or checks passing, run `factory run update <id> <stage> running --activity "<short phrase>"`. Keep the activity to one short line without secrets.
- **Rework:** when Check sends the run back to Build, run `factory run update <id> build running --rework <n> --activity "Rework <n>: <findings being fixed>"`, numbering Reworks from 1 within the run. Leave the count set when Check runs again.
- **Ticket:** when work on a ticket with an HTTP(S) URL starts, add `--current-ticket "<url>"` to that update. When no ticket is in progress anymore, add `--clear-current-ticket`. Leave it unset for local Markdown tickets.
- **Blocked:** when work cannot continue without outside input, run `factory run update <id> <stage> blocked --blocking-reason "<what is needed>"`. When work resumes, run `factory run update <id> <stage> running --clear-blocking-reason`.
- **Done:** once the pull request is open and its CI is green (or the project has no CI), run `factory run update <id> finish done --activity "Opened <pull request URL>" --clear-current-ticket`.

Options that are left out keep their current value. If the server or an update fails, report the failure rather than claiming progress is tracked.

## Workspace

After `factory run start`, give the run its own git worktree so several runs can share one repository without switching branches under each other. Never reuse or modify the original checkout, even if it is on the base branch or has uncommitted changes.

1. Determine the PR base branch: the project's documented base branch, otherwise the remote default from `git symbolic-ref refs/remotes/origin/HEAD`.
2. Run `git fetch origin <base>` so the run starts from the remote tip, not the local base branch.
3. Choose a short feature branch name for the task, then run `git worktree add -b <branch> <repo-parent>/<repo-name>.worktrees/<branch-with-slashes-as-dashes> origin/<base>`. If the branch or path already exists, choose a new name instead of reusing it.
4. If the worktree cannot be created, mark the run blocked with the reason and stop.

Run every later stage from this worktree: Define files, Build, checks, `factory run update` commands, commit, push, and the pull request.

## Define

Before anything else in Define, check that `docs/agents/issue-tracker.md` exists in the run's worktree. If it is missing, look in the original checkout the run was started from. If it exists there (setup was run but not committed to the base branch), copy that checkout's `docs/agents/` directory into the worktree and bring its `## Agent skills` section into the worktree's `AGENTS.md` or `CLAUDE.md` (whichever the original checkout edited), so the pull request includes the setup output; do not modify the original checkout. If it exists in neither place, the repository has not been set up for the factory: mark the run blocked with `--blocking-reason "Run /setup-matt-pocock-skills first"`, tell the user to run `/setup-matt-pocock-skills` once in the original checkout and then start the run again (the new worktree will pick up its output; this run's worktree can be removed), and stop. Do not guess a tracker.

1. Use `grill-with-docs` to clarify the request and record any useful domain decisions.
2. Use `to-spec` to write a concise spec from the discussion.
3. Use `to-tickets` to break the spec into the smallest useful set of tickets, including dependencies when needed.

Store the spec and tickets in the tracker configured in `docs/agents/issue-tracker.md`. Do not require extra approval of test seams or a separate ticket-breakdown approval. Ask the user when a decision is actually needed.

## Build

Use `implement-spec` to implement the tickets on the run's feature branch, building each ticket test-first with `tdd`. Work sequentially in the run's worktree; do not require subagents, extra worktrees beyond it, a draft PR, or a second code review here. Run the project's relevant checks and fix failures before Check. Find the project's verification: the commands its agent instructions (`AGENTS.md`, `CLAUDE.md`), contributing docs or build files say to run before merging, which should match what its CI runs. Run all of them after each ticket, not only the relevant ones; a missing tool or a failing check blocks the run, so fix it or mark the run blocked, and never skip it. If you cannot find any, ask the user once, or say so in the pull request when running unattended.

## Check

Use `code-review` against the branch's starting point and the spec, then use `security-review` to scan the changed code. When there are no actionable findings, go to Finish.

When there are actionable findings, start a Rework: return to Build, fix the findings test-first with `tdd`, run the relevant checks, then run Check again with both reviews. A run gets at most two Reworks.

Never block work on Check findings. When findings remain after the second Rework, or a finding cannot be resolved, keep the run running with `--activity "Waiting for a decision on remaining Check findings"`, list the remaining findings, and ask the user whether to do one more Rework, finish with the findings listed in the pull request, or stop. Follow their choice.

## Finish

Use `pr` to write the pull request description. When the user chose to finish with remaining Check findings, list them in the description. Commit the work. Run the project's verification on the final commit before pushing and report its result; do not open the pull request while a check fails or cannot run. Push the branch and create a pull request targeting the base branch chosen in Workspace. Then watch the pull request's CI: follow every check until all have completed, noticing failures as well as success, and keep the run running with `--activity "Waiting for CI"`. When a check fails, read its log, fix the cause in a new commit on the same branch (never amend or force-push), rerun the project's verification, push, note it with a milestone update, and watch the new run; repeat until CI is green. CI fixes are not Reworks. A failure you cannot fix, such as an infrastructure problem, a missing permission, or a flaky external dependency, blocks the run with that reason. When the project has no CI, skip the watch and say so. Never merge the pull request; that is the user's decision. Report the pull request as ready for review only when CI is green. Keep the worktree for review follow-ups. Report its link, the worktree path (removable with `git worktree remove <path>` after merge), the checks performed, and the final CI result. If a required tool, permission, or repository configuration is unavailable, report what remains, mark the run blocked with that as the reason, and stop.
