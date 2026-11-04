---
name: factory2
description: Take a software change from an idea to a pull request through define, build, check, and finish. Use when the user asks to run factory2.
---

# Factory 2

Take the user's request through these stages in order. Tell the user what each stage produced. Use the named skills, reading their `SKILL.md` files and any referenced resources when needed. In this project's checkout, the bundled upstream skills are in `skills/<name>/`; in an installed bundle, they are siblings of this directory. Otherwise use the corresponding installed skills. This workflow's instructions take precedence where they differ from those skills.

## Run reporting

Report progress to the factory dashboard with the `factory2` CLI from the project directory. Update only at the points below; do not send periodic updates.

- **Start:** before Define, run `factory2 run start "<short task title>"` and keep the printed run ID. If the server is not running, start `factory2 dashboard` as a background process, then retry once. If a different or outdated server occupies the port, or the CLI rejects an option below, report that it needs to be stopped or upgraded.
- **Stage:** at the start of each later stage, run `factory2 run update <id> <build|check|finish> running --activity "<what the stage is doing>"`.
- **Milestone:** after a meaningful milestone, such as tickets created or checks passing, run `factory2 run update <id> <stage> running --activity "<short phrase>"`. Keep the activity to one short line without secrets.
- **Ticket:** when work on a ticket with an HTTP(S) URL starts, add `--current-ticket "<url>"` to that update. When no ticket is in progress anymore, add `--clear-current-ticket`. Leave it unset for local Markdown tickets.
- **Blocked:** when work cannot continue without outside input, run `factory2 run update <id> <stage> blocked --blocking-reason "<what is needed>"`. When work resumes, run `factory2 run update <id> <stage> running --clear-blocking-reason`.
- **Done:** run `factory2 run update <id> finish done --activity "Opened <pull request URL>" --clear-current-ticket`.

Options that are left out keep their current value. If the server or an update fails, report the failure rather than claiming progress is tracked.

## Define

1. Use `grill-with-docs` to clarify the request and record any useful domain decisions.
2. Use `to-spec` to write a concise spec from the discussion.
3. Use `to-tickets` to break the spec into the smallest useful set of tickets, including dependencies when needed.

Keep the spec and tickets as local Markdown files when no tracker is configured; use the project's configured tracker when available. Do not require tracker setup, extra approval of test seams, or a separate ticket-breakdown approval. Ask the user when a decision is actually needed.

## Build

Use `implement-spec` to implement the tickets on a feature branch. Work sequentially in the current checkout; do not require TDD, subagents, extra worktrees, a draft PR, or a second code review here. Run the project's relevant checks and fix failures before Check.

## Check

Use `code-review` against the branch's starting point and the spec, then use `security-review` to scan the changed code. Fix actionable findings and rerun the relevant checks and reviews. If a finding cannot be resolved, report it, mark the run blocked with the finding as the reason, and stop before Finish.

## Finish

Use `pr` to write the pull request description. Commit the work, push the branch, and create a pull request targeting the project's default branch (`develop` for this project). Report its link and the checks performed. If a required tool, permission, or repository configuration is unavailable, report what remains, mark the run blocked with that as the reason, and stop.
