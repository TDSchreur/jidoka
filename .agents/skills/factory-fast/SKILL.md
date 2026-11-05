---
name: factory-fast
description: Take a software change from an idea to a pull request through define, build, and finish. Use when the user asks to run factory-fast.
---

# TDS Factory Fast

Take the user's request through these stages in order. Keep stage updates brief. Load only the skills needed for the task, reading their `SKILL.md` files and relevant referenced resources. In this project's checkout, bundled implementation and planning skills are in `skills/<name>/`; in an installed bundle, they are siblings of this directory. Otherwise use the corresponding installed skills. This workflow's instructions take precedence where they differ from those skills.

## Run reporting

Report progress to the factory dashboard with the `factory` CLI from the run's worktree (see Workspace). Update only at the points below; do not send periodic updates.

- **Start:** before Define, run `factory run start "<short task title>"` and keep the printed run ID. If the server is not running, start `factory dashboard` as a background process, then retry once. If a different or outdated server occupies the port, or the CLI rejects an option below, report that it needs to be stopped or upgraded.
- **Stage:** at the start of each later stage, run `factory run update <id> <build|finish> running --activity "<what the stage is doing>"`.
- **Milestone:** after a meaningful milestone, such as tickets created or checks passing, run `factory run update <id> <stage> running --activity "<short phrase>"`. Keep the activity to one short line without secrets.
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

For a clear, bounded request, use the request as the spec and state a short implementation plan and acceptance criteria in the conversation. Continue directly to Build without separate spec files, tickets, or an approval round.

Use `grill-with-docs` only when unresolved requirements or domain decisions materially affect implementation. Use `to-spec` when the task needs a durable spec, and `to-tickets` when multiple dependent pieces benefit from explicit tracking. Reuse existing specs and tickets rather than recreating them.

When these artifacts are needed, keep them as local Markdown files if no tracker is configured; otherwise use the project's configured tracker. Do not require tracker setup, extra approval of test seams, or a separate ticket-breakdown approval. Ask the user when a decision is actually needed.

## Build

Use `implement-spec-fast` to implement the request or tickets on the run's feature branch without TDD. Work sequentially in the run's worktree; do not require subagents, extra worktrees beyond it, a draft PR, or a code review here. Run the project's relevant checks and fix failures before Finish. Reuse passing check results while the checked code remains unchanged; repeat checks when later changes affect their coverage. Always honor checks required by project instructions. When the project has `scripts/verify.sh`, run it before Finish; a missing tool or a failing check blocks the run, so fix it or mark the run blocked, and never skip it.

## Finish

Use `pr` to write the pull request description. Commit the work. When the project has `scripts/verify.sh`, run it on the final commit before pushing and report its result; do not open the pull request while it fails or cannot run. Push the branch and create a pull request targeting the project's default branch (`develop` for this project). Then watch the pull request's CI: follow every check until all have completed, noticing failures as well as success, and keep the run running with `--activity "Waiting for CI"`. When a check fails, read its log, fix the cause in a new commit on the same branch (never amend or force-push), run `scripts/verify.sh` when the project has it, push, note it with a milestone update, and watch the new run; repeat until CI is green. CI fixes are not Reworks. A failure you cannot fix, such as an infrastructure problem, a missing permission, or a flaky external dependency, blocks the run with that reason. When the project has no CI, skip the watch and say so. Never merge the pull request; that is the user's decision. Report the pull request as ready for review only when CI is green. Keep the worktree for review follow-ups. Report its link, the worktree path (removable with `git worktree remove <path>` after merge), the checks performed, and the final CI result. If a required tool, permission, or repository configuration is unavailable, report what remains, mark the run blocked with that as the reason, and stop.
