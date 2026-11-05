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
- **Done:** run `factory run update <id> finish done --activity "Opened <pull request URL>" --clear-current-ticket`.

Options that are left out keep their current value. If the server or an update fails, report the failure rather than claiming progress is tracked.

## Workspace

After `factory run start`, give the run its own git worktree so several runs can share one repository without switching branches under each other. Never reuse or modify the original checkout, even if it is on the base branch or has uncommitted changes.

1. Determine the PR base branch: the project's documented base branch, otherwise the remote default from `git symbolic-ref refs/remotes/origin/HEAD`.
2. Run `git fetch origin <base>` so the run starts from the remote tip, not the local base branch.
3. Choose a short feature branch name for the task, then run `git worktree add -b <branch> <repo-parent>/<repo-name>.worktrees/<branch-with-slashes-as-dashes> origin/<base>`. If the branch or path already exists, choose a new name instead of reusing it.
4. If the worktree cannot be created, mark the run blocked with the reason and stop.

Run every later stage from this worktree: Define files, Build, checks, `factory run update` commands, commit, push, and the pull request.

## Define

1. Use `grill-with-docs` to clarify the request and record any useful domain decisions.
2. Use `to-spec` to write a concise spec from the discussion.
3. Use `to-tickets` to break the spec into the smallest useful set of tickets, including dependencies when needed.

Keep the spec and tickets as local Markdown files when no tracker is configured; use the project's configured tracker when available. Do not require tracker setup, extra approval of test seams, or a separate ticket-breakdown approval. Ask the user when a decision is actually needed.

## Build

Use `implement-spec` to implement the tickets on the run's feature branch, building each ticket test-first with `tdd`. Work sequentially in the run's worktree; do not require subagents, extra worktrees beyond it, a draft PR, or a second code review here. Run the project's relevant checks and fix failures before Check. When the project has `scripts/verify.sh`, run it after each ticket; a missing tool or a failing check blocks the run, so fix it or mark the run blocked, and never skip it.

## Check

Use `code-review` against the branch's starting point and the spec, then use `security-review` to scan the changed code. When there are no actionable findings, go to Finish.

When there are actionable findings, start a Rework: return to Build, fix the findings test-first with `tdd`, run the relevant checks, then run Check again with both reviews. A run gets at most two Reworks.

Never block work on Check findings. When findings remain after the second Rework, or a finding cannot be resolved, keep the run running with `--activity "Waiting for a decision on remaining Check findings"`, list the remaining findings, and ask the user whether to do one more Rework, finish with the findings listed in the pull request, or stop. Follow their choice.

## Finish

Use `pr` to write the pull request description. When the user chose to finish with remaining Check findings, list them in the description. Commit the work. When the project has `scripts/verify.sh`, run it on the final commit before pushing and report its result; do not open the pull request while it fails or cannot run. Push the branch and create a pull request targeting the project's default branch (`develop` for this project). Keep the worktree for review follow-ups. Report its link, the worktree path (removable with `git worktree remove <path>` after merge), and the checks performed. If a required tool, permission, or repository configuration is unavailable, report what remains, mark the run blocked with that as the reason, and stop.
