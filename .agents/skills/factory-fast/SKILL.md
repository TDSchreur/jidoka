---
name: factory-fast
description: Take a software change from an idea to a pull request through define, build, and finish. Use when the user asks to run factory-fast.
---

# TDS Factory Fast

Take the user's request through these stages in order. Keep stage updates brief. Load only the skills needed for the task, reading their `SKILL.md` files and relevant referenced resources. In this project's checkout, bundled implementation and planning skills are in `skills/<name>/`; in an installed bundle, they are siblings of this directory. Otherwise use the corresponding installed skills. This workflow's instructions take precedence where they differ from those skills. Read [factory-procedures](../factory-procedures/SKILL.md) before starting: it holds the procedures shared by all three workflows (run reporting, continuing a run, workspace, tracker setup, pull request and CI).

## Run reporting

Report progress to the factory dashboard as described under Run reporting in [factory-procedures](../factory-procedures/SKILL.md), with `--mode factory-fast`. This workflow has the stages define, build and finish.

## Continuing a run

To continue an existing run, follow Continuing a run in [factory-procedures](../factory-procedures/SKILL.md).

## Workspace

Follow Workspace in [factory-procedures](../factory-procedures/SKILL.md): every run gets its own fresh worktree, and the PR base branch chosen there is used in Finish.

## Writing

Follow Writing in [factory-procedures](../factory-procedures/SKILL.md) for all human-facing prose in every stage.

## Define

For a clear, bounded request, use the request as the spec and state a short implementation plan and acceptance criteria in the conversation; no tracker setup is needed for that. Save them as described in Saving the plan in [factory-procedures](../factory-procedures/SKILL.md), then continue directly to Build without separate spec files, tickets, or an approval round.

Use `grill-with-docs` only when unresolved requirements or domain decisions materially affect implementation. Use `to-spec` when the task needs a durable spec, and `to-tickets` when multiple dependent pieces benefit from explicit tracking. Reuse existing specs and tickets rather than recreating them. Before reading or publishing specs or tickets in the tracker, follow Tracker setup in [factory-procedures](../factory-procedures/SKILL.md).

When these artifacts are needed, store them in the tracker configured in `docs/agents/issue-tracker.md`. Do not require extra approval of test seams or a separate ticket-breakdown approval. Ask the user when a decision is actually needed.

## Build

Use `implement-spec-fast` to implement the request or tickets on the run's feature branch without TDD. Work sequentially in the run's worktree; do not require subagents, extra worktrees beyond it, a draft PR, or a code review here. Run the project's relevant checks as you go and fix failures before moving on. Reuse a passing result only while the checked code and its relevant inputs are unchanged, and repeat checks when later changes affect their coverage. Always honor stricter checks the project requires. Find the project's verification: the commands its agent instructions (`AGENTS.md`, `CLAUDE.md`), contributing docs or build files say to run before merging, which should match what its CI runs. Run all of them once, after the last change and before Finish, not only the relevant ones; a missing tool or a failing check blocks the run, so fix it or mark the run blocked, and never skip it. If you cannot find any, ask the user once, or say so in the pull request when running unattended.

## Finish

Use `pr` to write the pull request description. Commit the work. Verify the final commit before pushing: reuse the latest passing full verification when nothing has changed since it ran (same `HEAD`, clean tree), otherwise run the project's verification on it. Report the result; do not open the pull request while a check fails or cannot run. Then follow Pull request and CI in [factory-procedures](../factory-procedures/SKILL.md).
