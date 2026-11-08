---
name: factory-full
description: Test-first factory workflow with verification after each ticket and both reviews. Usually entered from `factory`; use directly only when the user names factory-full.
---

# TDS Factory Full

Take the user's request through these stages in order. Tell the user what each stage produced. Use the named skills, reading their `SKILL.md` files and any referenced resources when needed. Using the bundled skills in the shared procedures says where they are, which of them only the user can invoke, and that this workflow's instructions take precedence over them. `factory-implement` can also only be invoked by the user: read its `SKILL.md` from that location instead of invoking it. Read [factory-procedures](../factory-procedures/SKILL.md) before starting: it holds the procedures shared by all three workflows (using the bundled skills, run reporting, continuing a run, workspace, tracker setup, finish, pull request and CI).

## Shared procedures

Follow Run reporting, Continuing a run (to continue an existing run) and Workspace in [factory-procedures](../factory-procedures/SKILL.md). Report with `--mode factory-full`; this workflow has the stages define, build, check and finish. Every run gets its own fresh worktree, and the PR base branch chosen in Workspace is used in Finish.

## Define

Before anything else in Define, follow Tracker setup in [factory-procedures](../factory-procedures/SKILL.md). Then read [Model routing](../factory-procedures/references/model-routing.md) and follow it for Define.

1. Use `factory-grill-with-docs` to clarify the request and record any useful domain decisions.
2. Use `factory-to-spec` to write a concise spec from the discussion.
3. Use `factory-to-tickets` to break the spec into the smallest useful set of tickets, including dependencies when needed.

Store the spec and tickets in the tracker configured in `docs/agents/issue-tracker.md`.

Follow Define rules in [factory-procedures](../factory-procedures/SKILL.md) for the whole Define stage.

## Build

When Build is routed (see [Model routing](../factory-procedures/references/model-routing.md)), hand each ticket, and a Rework's fixes, to a fresh agent on the build model as it describes; this takes precedence over the build skill's "do not require subagents".

Use `factory-implement` to implement the tickets on the run's feature branch, building each ticket test-first with `factory-tdd`. Work sequentially in the run's worktree. Run the project's relevant checks and fix failures before moving on. Run the full verification after every ticket; follow Verification in [factory-procedures](../factory-procedures/SKILL.md).

## Check

Before Check, follow Committing for Check in [factory-procedures](../factory-procedures/SKILL.md).

Use `factory-code-review` with the base commit as the fixed point, and with the spec, then use `factory-security-review` on the same range (base commit to `HEAD`). Follow [Model routing](../factory-procedures/references/model-routing.md) for Check: it says which model and effort `factory-code-review`'s agents use. When there are no actionable findings, go to Finish.

When there are actionable findings, start a Rework: return to Build, fix the findings test-first with `factory-tdd`, run the relevant checks and then the full verification, and commit the fixes together with any new files. Then run Check again with both reviews (`factory-code-review` and `factory-security-review`), each using the previously reviewed commit as the fixed point so the review covers exactly the Rework's changes, together with the findings it addressed. Pin the new `HEAD` as the reviewed commit, replacing the earlier pin. Follow Reworks in [factory-procedures](../factory-procedures/SKILL.md).

## Finish

Follow Finish in [factory-procedures](../factory-procedures/SKILL.md). The implementation and Reworks are already committed.
