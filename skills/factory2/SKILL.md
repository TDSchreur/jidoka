---
name: factory2
description: Take a software change from an idea to a pull request through define, build, check, and finish. Use when the user asks to run factory2.
---

# Factory 2

Take the user's request through these stages in order. Tell the user what each stage produced. Use the named skills, reading their `SKILL.md` files and any referenced resources when needed. They are siblings of this directory; if installed separately, use the corresponding installed skills. This workflow's instructions take precedence where they differ from those skills.

Before Define, run `factory2 run start "<short task title>"` from the project directory and retain the printed run ID. If the server is not running, start `factory2 dashboard` as a background process, then retry once. If a different or outdated server occupies the port, report that it needs to be stopped or upgraded. At the start of each later stage run `factory2 run update <id> <build|check|finish> running`. On completion, update it with `factory2 run update <id> finish done`. When work is blocked, update the current stage with status `blocked`; set it back to `running` when work resumes. If the server or update fails, report the failure rather than claiming progress is tracked.

## Define

1. Use `grill-with-docs` to clarify the request and record any useful domain decisions.
2. Use `to-spec` to write a concise spec from the discussion.
3. Use `to-tickets` to break the spec into the smallest useful set of tickets, including dependencies when needed.

Keep the spec and tickets as local Markdown files when no tracker is configured; use the project's configured tracker when available. Do not require tracker setup, extra approval of test seams, or a separate ticket-breakdown approval. Ask the user when a decision is actually needed.

## Build

Use `implement-spec` to implement the tickets on a feature branch. Work sequentially in the current checkout; do not require TDD, subagents, extra worktrees, a draft PR, or a second code review here. Run the project's relevant checks and fix failures before Check.

## Check

Use `code-review` against the branch's starting point and the spec, then use `security-review` to scan the changed code. Fix actionable findings and rerun the relevant checks and reviews. If a finding cannot be resolved, report it and stop before Finish.

## Finish

Use `pr` to write the pull request description. Commit the work, push the branch, and create a pull request targeting the project's default branch (`develop` for this project). Report its link and the checks performed. If a required tool, permission, or repository configuration is unavailable, report what remains and stop.
