---
name: jidoka-balanced
description: Lightweight-planning jidoka workflow with proportionate tests and one initial code review. Usually entered from `jidoka`; use directly only when the user names jidoka-balanced.
---

# Jidoka Balanced

Take the user's request through these stages in order. Keep stage updates brief. Load only the skills needed for the task, reading their `SKILL.md` files and relevant referenced resources. Using the bundled skills in the shared procedures says where they are, which of them only the user can invoke, and that this workflow's instructions take precedence over them. Read [jidoka-procedures](../jidoka-procedures/SKILL.md) before starting: it holds the procedures shared by all three workflows (using the bundled skills, run reporting, continuing a run, workspace, tracker setup, finish, pull request and CI).

## Shared procedures

Follow Run ownership, Run reporting, Continuing a run (to continue an existing run) and Workspace in [jidoka-procedures](../jidoka-procedures/SKILL.md). Report with `--mode jidoka-balanced`; this workflow has the stages define, build, check and finish. Every run gets its own fresh worktree, and the PR base branch chosen in Workspace is used in Finish.

## Define

When the request is clear and bounded, use it as the spec and state the plan and explicit acceptance criteria in the conversation; no tracker setup is needed for that. Otherwise follow Tracker setup in [jidoka-procedures](../jidoka-procedures/SKILL.md) and use `jidoka-to-spec` once to write a concise spec from the request and the code, without interviewing the user, stating explicit acceptance criteria either way. Before Build, if the plan and criteria exist only in the conversation, follow Saving the plan in [jidoka-procedures](../jidoka-procedures/SKILL.md).

Use `jidoka-to-tickets` only when the work has multiple pieces that benefit from separate tracking. Use `jidoka-grill-with-docs` only when an ambiguity blocks implementation, and limit it to that ambiguity. Reuse existing specs and tickets rather than recreating them. Reading or publishing specs and tickets in the tracker needs Tracker setup first.

When these artifacts are needed, store them in the tracker configured in `docs/agents/issue-tracker.md`.

Follow Define rules in [jidoka-procedures](../jidoka-procedures/SKILL.md) for the whole Define stage.

## Build

When Build is routed (see [Model routing](../jidoka-procedures/references/model-routing.md)), hand each ticket, and a Rework's fixes, to a fresh agent on the build model as it describes; this takes precedence over the build skill's "do not require subagents".

Use `jidoka-implement-balanced` to implement the request or tickets on the run's feature branch: implement each ticket and add behavior tests as appropriate under project instructions, without red-green cycles or seam approval. Work sequentially in the run's worktree; do not require other subagents, extra worktrees beyond it, a draft PR, or a code review here. Use targeted or quick checks during iteration, following the project’s requirements; follow Verification in [jidoka-procedures](../jidoka-procedures/SKILL.md).

## Check

Before Check, follow Committing for Check in [jidoka-procedures](../jidoka-procedures/SKILL.md).

Use one `jidoka-code-review` pass with the base commit as the fixed point, and with the spec or acceptance criteria (the recorded spec or saved plan; pass a saved plan's absolute path as the spec source, which needs no tracker). Use `jidoka-security-review` on the same range when the change meaningfully affects a security boundary or sensitive data flow, such as authorization, handling secrets or untrusted input across a boundary; a touched file or dependency alone is not sufficient. Otherwise skip it and say why. Honor an explicit user request for a security review. Follow [Model routing](../jidoka-procedures/references/model-routing.md) for Check: it says which model and effort `jidoka-code-review`'s agents use.

When there are no actionable findings, go to Finish. Otherwise start a Rework: return to Build, fix the findings directly, run appropriate checks, and commit the fixes including new files. The coordinator checks small, clear fixes against the findings. For substantial or uncertain fixes, repeat a scoped `jidoka-code-review` from the previously reviewed commit, including the findings addressed; repeat the security review when those fixes meaningfully affect a security boundary or sensitive data flow, or the user requests it. Record how the fixes were checked and pin the new `HEAD`. Follow Reworks in [jidoka-procedures](../jidoka-procedures/SKILL.md).

## Finish

Follow Finish in [jidoka-procedures](../jidoka-procedures/SKILL.md). The implementation and Reworks are already committed.
