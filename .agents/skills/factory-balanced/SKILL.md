---
name: factory-balanced
description: Lightweight-planning factory workflow with tests after implementation and one code review, plus a review of each Rework. Usually entered from `factory`; use directly only when the user names factory-balanced.
---

# TDS Factory Balanced

Take the user's request through these stages in order. Keep stage updates brief. Load only the skills needed for the task, reading their `SKILL.md` files and relevant referenced resources. Using the bundled skills in the shared procedures says where they are, which of them only the user can invoke, and that this workflow's instructions take precedence over them. Read [factory-procedures](../factory-procedures/SKILL.md) before starting: it holds the procedures shared by all three workflows (using the bundled skills, run reporting, continuing a run, workspace, tracker setup, finish, pull request and CI).

## Shared procedures

Follow Run ownership, Run reporting, Continuing a run (to continue an existing run) and Workspace in [factory-procedures](../factory-procedures/SKILL.md). Report with `--mode factory-balanced`; this workflow has the stages define, build, check and finish. Every run gets its own fresh worktree, and the PR base branch chosen in Workspace is used in Finish.

## Define

When the request is clear and bounded, use it as the spec and state the plan and explicit acceptance criteria in the conversation; no tracker setup is needed for that. Otherwise follow Tracker setup in [factory-procedures](../factory-procedures/SKILL.md) and use `factory-to-spec` once to write a concise spec from the request and the code, without interviewing the user, stating explicit acceptance criteria either way. Before Build, if the plan and criteria exist only in the conversation, follow Saving the plan in [factory-procedures](../factory-procedures/SKILL.md).

Use `factory-to-tickets` only when the work has multiple pieces that benefit from separate tracking. Use `factory-grill-with-docs` only when an ambiguity blocks implementation, and limit it to that ambiguity. Reuse existing specs and tickets rather than recreating them. Reading or publishing specs and tickets in the tracker needs Tracker setup first.

When these artifacts are needed, store them in the tracker configured in `docs/agents/issue-tracker.md`.

Follow Define rules in [factory-procedures](../factory-procedures/SKILL.md) for the whole Define stage.

## Build

When Build is routed (see [Model routing](../factory-procedures/references/model-routing.md)), hand each ticket, and a Rework's fixes, to a fresh agent on the build model as it describes; this takes precedence over the build skill's "do not require subagents".

Use `factory-implement-balanced` to implement the request or tickets on the run's feature branch: implement each ticket, then add behavior tests through public interfaces without red-green cycles or seam approval. Work sequentially in the run's worktree; do not require other subagents, extra worktrees beyond it, a draft PR, or a code review here. Run the project's relevant checks for each ticket and fix failures before moving on. Run the full verification once after the last ticket on the candidate for Check; follow Verification in [factory-procedures](../factory-procedures/SKILL.md).

## Check

Before Check, follow Committing for Check in [factory-procedures](../factory-procedures/SKILL.md).

Use one `factory-code-review` pass with the base commit as the fixed point, and with the spec or acceptance criteria (the recorded spec or saved plan; pass a saved plan's absolute path as the spec source, which needs no tracker). Use `factory-security-review` on the same range only when the diff touches input parsing, authentication or secrets, network access, file or process handling, or dependencies; otherwise skip it and say why. Follow [Model routing](../factory-procedures/references/model-routing.md) for Check: it says which model and effort `factory-code-review`'s agents use.

When there are no actionable findings, go to Finish. When there are, start a Rework: return to Build, fix the findings with `factory-implement-balanced` (implement, then add or adjust behavior tests), run the relevant checks and then the full verification on the changed code, and commit the fixes together with any new files. Then run Check again with one `factory-code-review` pass using the previously reviewed commit as the fixed point, so the review covers exactly the Rework's changes, together with the findings it addressed, plus `factory-security-review` over the same range when those changes meet the criteria above. Pin the new `HEAD` as the reviewed commit, replacing the earlier pin. Follow Reworks in [factory-procedures](../factory-procedures/SKILL.md).

## Finish

Follow Finish in [factory-procedures](../factory-procedures/SKILL.md). The implementation and Reworks are already committed.
