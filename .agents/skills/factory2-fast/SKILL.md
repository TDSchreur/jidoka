---
name: factory2-fast
description: Take a software change from an idea to a pull request through define, build, and finish. Use when the user asks to run factory2-fast.
---

# Factory 2 Fast

Take the user's request through these stages in order. Keep stage updates brief. Load only the skills needed for the task, reading their `SKILL.md` files and relevant referenced resources. In this project's checkout, bundled implementation and planning skills are in `skills/<name>/`; in an installed bundle, they are siblings of this directory. Otherwise use the corresponding installed skills. This workflow's instructions take precedence where they differ from those skills.

Before Define, run `factory2 run start "<short task title>"` from the project directory and retain the printed run ID. If the server is not running, start `factory2 dashboard` as a background process, then retry once. If a different or outdated server occupies the port, report that it needs to be stopped or upgraded. At the start of each later stage run `factory2 run update <id> <build|finish> running`. On completion, update it with `factory2 run update <id> finish done`. When work is blocked, update the current stage with status `blocked`; set it back to `running` when work resumes. If the server or update fails, report the failure rather than claiming progress is tracked.

## Define

For a clear, bounded request, use the request as the spec and state a short implementation plan and acceptance criteria in the conversation. Continue directly to Build without separate spec files, tickets, or an approval round.

Use `grill-with-docs` only when unresolved requirements or domain decisions materially affect implementation. Use `to-spec` when the task needs a durable spec, and `to-tickets` when multiple dependent pieces benefit from explicit tracking. Reuse existing specs and tickets rather than recreating them.

When these artifacts are needed, keep them as local Markdown files if no tracker is configured; otherwise use the project's configured tracker. Do not require tracker setup, extra approval of test seams, or a separate ticket-breakdown approval. Ask the user when a decision is actually needed.

## Build

Use `implement-spec-fast` to implement the request or tickets on a feature branch without TDD. Work sequentially in the current checkout; do not require subagents, extra worktrees, a draft PR, or a code review here. Run the project's relevant checks and fix failures before Finish. Reuse passing check results while the checked code remains unchanged; repeat checks when later changes affect their coverage. Always honor checks required by project instructions.

## Finish

Use `pr` to write the pull request description. Commit the work, push the branch, and create a pull request targeting the project's default branch (`develop` for this project). Report its link and the checks performed. If a required tool, permission, or repository configuration is unavailable, report what remains and stop.
