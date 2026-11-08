---
name: factory-fast
description: Fast factory workflow without a Check stage. Usually entered from `factory`; use directly only when the user names factory-fast.
---

# TDS Factory Fast

Take the user's request through these stages in order. Keep stage updates brief. Load only the skills needed for the task, reading their `SKILL.md` files and relevant referenced resources. Using the bundled skills in the shared procedures says where they are, which of them only the user can invoke, and that this workflow's instructions take precedence over them. Read [factory-procedures](../factory-procedures/SKILL.md) before starting: it holds the procedures shared by all three workflows (using the bundled skills, run reporting, continuing a run, workspace, tracker setup, finish, pull request and CI).

## Shared procedures

Follow Run reporting, Continuing a run (to continue an existing run) and Workspace in [factory-procedures](../factory-procedures/SKILL.md). Report with `--mode factory-fast`; this workflow has the stages define, build and finish. Every run gets its own fresh worktree, and the PR base branch chosen in Workspace is used in Finish.

## Define

For a clear, bounded request, use the request as the spec and state a short implementation plan and acceptance criteria in the conversation; no tracker setup is needed for that. Save them as described in Saving the plan in [factory-procedures](../factory-procedures/SKILL.md), then continue directly to Build without separate spec files, tickets, or an approval round.

Use `factory-grill-with-docs` only when unresolved requirements or domain decisions materially affect implementation. Use `factory-to-spec` when the task needs a durable spec, and `factory-to-tickets` when multiple dependent pieces benefit from explicit tracking. Reuse existing specs and tickets rather than recreating them. Before reading or publishing specs or tickets in the tracker, follow Tracker setup in [factory-procedures](../factory-procedures/SKILL.md).

When these artifacts are needed, store them in the tracker configured in `docs/agents/issue-tracker.md`. Follow Define rules in [factory-procedures](../factory-procedures/SKILL.md).

## Build

When Build is routed (see [Model routing](../factory-procedures/references/model-routing.md)), hand each ticket to a fresh agent on the build model as it describes; this takes precedence over the build skill's "do not require subagents".

Use `factory-implement-fast` to implement the request or tickets on the run's feature branch without TDD. Work sequentially in the run's worktree; do not require other subagents, extra worktrees beyond it, a draft PR, or a code review here. Run the project's relevant checks as you go and fix failures before moving on. Run the full verification once after the last change and before Finish; follow Verification in [factory-procedures](../factory-procedures/SKILL.md).

## Finish

Follow Finish in [factory-procedures](../factory-procedures/SKILL.md). Nothing is committed before Finish here, so commit the work there.
