---
name: factory-full
description: Test-first factory workflow with verification after each ticket and both reviews. Usually entered from `factory`; use directly only when the user names factory-full.
---

# TDS Factory Full

Take the user's request through these stages in order. Tell the user what each stage produced. Use the named skills, reading their `SKILL.md` files and any referenced resources when needed. In this project's checkout, the bundled upstream skills are in `skills/<name>/`; in an installed bundle, they are siblings of this directory. Otherwise use the corresponding installed skills. `factory-grill-with-docs`, `factory-to-spec`, `factory-to-tickets` and `factory-implement` can only be invoked by the user, so the agent's skill tool may refuse them: read their `SKILL.md` from that location instead of invoking them. This workflow's instructions take precedence where they differ from those skills. Read [factory-procedures](../factory-procedures/SKILL.md) before starting: it holds the procedures shared by all three workflows (run reporting, continuing a run, workspace, tracker setup, pull request and CI).

## Run reporting

Report progress to the factory dashboard as described under Run reporting in [factory-procedures](../factory-procedures/SKILL.md), with `--mode factory-full`. This workflow has the stages define, build, check and finish.


## Continuing a run

To continue an existing run, follow Continuing a run in [factory-procedures](../factory-procedures/SKILL.md).

## Workspace

Follow Workspace in [factory-procedures](../factory-procedures/SKILL.md): every run gets its own fresh worktree, and the PR base branch chosen there is used in Finish.

## Define

Before anything else in Define, follow Tracker setup in [factory-procedures](../factory-procedures/SKILL.md). Then read [Model routing](../factory-procedures/references/model-routing.md) and follow it for Define.

1. Use `factory-grill-with-docs` to clarify the request and record any useful domain decisions.
2. Use `factory-to-spec` to write a concise spec from the discussion.
3. Use `factory-to-tickets` to break the spec into the smallest useful set of tickets, including dependencies when needed.

Store the spec and tickets in the tracker configured in `docs/agents/issue-tracker.md`. Do not require extra approval of test seams or a separate ticket-breakdown approval. Ask the user when a decision is actually needed.

## Build

At the start of Build and of every Rework, read [Model routing](../factory-procedures/references/model-routing.md) and follow it. When Build is routed, hand each ticket, and a Rework's fixes, to a fresh agent on the build model as it describes; this takes precedence over the build skill's "do not require subagents".

Use `factory-implement` to implement the tickets on the run's feature branch, building each ticket test-first with `factory-tdd`. Work sequentially in the run's worktree. Run the project's relevant checks and fix failures before moving on. Run the full verification after every ticket; follow Verification in [factory-procedures](../factory-procedures/SKILL.md).

## Check

Before Check, follow Committing for Check in [factory-procedures](../factory-procedures/SKILL.md).

Use `factory-code-review` with the base commit as the fixed point, and with the spec, then use `factory-security-review` on the same range (base commit to `HEAD`). At the start of every Check pass, including after a Rework, read [Model routing](../factory-procedures/references/model-routing.md) and follow it for Check: it says which model and effort `factory-code-review`'s agents use. When there are no actionable findings, go to Finish.

When there are actionable findings, start a Rework: return to Build, fix the findings test-first with `factory-tdd`, run the relevant checks and then the full verification, and commit the fixes together with any new files. Then run Check again with both reviews (`factory-code-review` and `factory-security-review`), each using the previously reviewed commit as the fixed point so the review covers exactly the Rework's changes, together with the findings it addressed. Pin the new `HEAD` as the reviewed commit, replacing the earlier pin. Follow Reworks in [factory-procedures](../factory-procedures/SKILL.md).


## Finish

At the start of Finish, read [Model routing](../factory-procedures/references/model-routing.md) and follow it for Finish.

Use `factory-pr` to write the pull request description. When the user chose to finish with remaining Check findings, list them in the description. The implementation and Reworks are already committed; commit anything still uncommitted that belongs to the change as a new commit. Verify the final commit before pushing: reuse the latest passing full verification when nothing has changed since it ran (same `HEAD`, clean tree), otherwise run the project's verification on it. Report the result; do not open the pull request while a check fails or cannot run. Then follow Pull request and CI in [factory-procedures](../factory-procedures/SKILL.md). The Finish report tells the user to run `factory run cleanup <id>` once the pull request merges.
