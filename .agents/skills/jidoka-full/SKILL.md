---
name: jidoka-full
description: Detailed jidoka workflow with project-directed testing and risk-based reviews. Usually entered from `jidoka`; use directly only when the user names jidoka-full.
---

# Jidoka Full

Take the user's request through these stages in order. Tell the user what each stage produced. Use the named skills, reading their `SKILL.md` files and any referenced resources when needed. Using the bundled skills in the shared procedures says where they are, which of them only the user can invoke, and that this workflow's instructions take precedence over them. `jidoka-implement` can also only be invoked by the user: read its `SKILL.md` from that location instead of invoking it. Read [jidoka-procedures](../jidoka-procedures/SKILL.md) before starting: it holds the procedures shared by all three workflows (using the bundled skills, run reporting, continuing a run, workspace, tracker setup, finish, pull request and CI).

## Shared procedures

Follow Run ownership, Run reporting, Continuing a run (to continue an existing run) and Workspace in [jidoka-procedures](../jidoka-procedures/SKILL.md). Report with `--mode jidoka-full`; this workflow has the stages define, build, check and finish. Every run gets its own fresh worktree, and the PR base branch chosen in Workspace is used in Finish.

## Define

Before anything else in Define, follow Tracker setup in [jidoka-procedures](../jidoka-procedures/SKILL.md). Then read [Model routing](../jidoka-procedures/references/model-routing.md) and follow it for Define.

1. Use `jidoka-grill-with-docs` to clarify the request and record any useful domain decisions.
2. Use `jidoka-to-spec` to write a concise spec from the discussion.
3. Use `jidoka-to-tickets` to break the spec into the smallest useful set of tickets, including dependencies when needed.

Store the spec and tickets in the tracker configured in `docs/agents/issue-tracker.md`.

Follow Define rules in [jidoka-procedures](../jidoka-procedures/SKILL.md) for the whole Define stage.

## Build

When Build is routed (see [Model routing](../jidoka-procedures/references/model-routing.md)), hand each ticket, and a Rework's fixes, to a fresh agent on the build model as it describes; this takes precedence over the build skill's "do not require subagents".

Use `jidoka-implement` to implement the tickets on the run's feature branch. Use test-first development when allowed by project instructions; otherwise follow the project's test approach. Work sequentially in the run's worktree. Use targeted or quick checks during iteration, following the project's requirements; follow Verification in [jidoka-procedures](../jidoka-procedures/SKILL.md).

## Check

Before Check, follow Committing for Check in [jidoka-procedures](../jidoka-procedures/SKILL.md).

Use `jidoka-code-review` with the base commit as the fixed point, and with the spec. Use `jidoka-security-review` on the same range when the change meaningfully affects a security boundary or sensitive data flow, such as authorization or handling secrets or untrusted input across a boundary; a touched file or dependency alone is not sufficient. Otherwise skip it and say why. Honor an explicit user request for a security review. Follow [Model routing](../jidoka-procedures/references/model-routing.md) for Check: it says which model and effort `jidoka-code-review`'s agents use. When there are no actionable findings, go to Finish.

When there are actionable findings, start a Rework: return to Build, fix the findings using the project’s test approach, run appropriate checks, and commit the fixes together with any new files. The coordinator checks small, clear fixes against the findings. For substantial or uncertain fixes, repeat a scoped `jidoka-code-review` from the previously reviewed commit, including the findings addressed; repeat the security review when those fixes meaningfully affect a security boundary or sensitive data flow, or the user requests it. Record how the fixes were checked and pin the new `HEAD`. Follow Reworks in [jidoka-procedures](../jidoka-procedures/SKILL.md).

## Finish

Follow Finish in [jidoka-procedures](../jidoka-procedures/SKILL.md). The implementation and Reworks are already committed.
