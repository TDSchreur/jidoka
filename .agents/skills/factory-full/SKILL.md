---
name: factory-full
description: Take a software change from an idea to a pull request through define, build, check, and finish. Use when the user asks to run factory-full.
---

# TDS Factory Full

Take the user's request through these stages in order. Tell the user what each stage produced. Use the named skills, reading their `SKILL.md` files and any referenced resources when needed. In this project's checkout, the bundled upstream skills are in `skills/<name>/`; in an installed bundle, they are siblings of this directory. Otherwise use the corresponding installed skills. This workflow's instructions take precedence where they differ from those skills. Read [factory-procedures](../factory-procedures/SKILL.md) before starting: it holds the procedures shared by all three workflows (run reporting, continuing a run, workspace, tracker setup, pull request and CI).

## Run reporting

Report progress to the factory dashboard as described under Run reporting in [factory-procedures](../factory-procedures/SKILL.md), with `--mode factory-full`. This workflow has the stages define, build, check and finish.

When Check sends the run back to Build, report it as a Rework: `factory run update <id> build running --rework <n> --activity "Rework <n>: <findings being fixed>"`, numbering Reworks from 1 within the run and leaving the count set when Check runs again.

## Continuing a run

To continue an existing run, follow Continuing a run in [factory-procedures](../factory-procedures/SKILL.md).

## Workspace

Follow Workspace in [factory-procedures](../factory-procedures/SKILL.md): every run gets its own fresh worktree, and the PR base branch chosen there is used in Finish.

## Writing

Follow Writing in [factory-procedures](../factory-procedures/SKILL.md) for all human-facing prose in every stage. Load `humanizer` right before the first text that section lists, and not when the run starts.

## Define

Before anything else in Define, follow Tracker setup in [factory-procedures](../factory-procedures/SKILL.md).

1. Use `grill-with-docs` to clarify the request and record any useful domain decisions.
2. Use `to-spec` to write a concise spec from the discussion.
3. Use `to-tickets` to break the spec into the smallest useful set of tickets, including dependencies when needed.

Store the spec and tickets in the tracker configured in `docs/agents/issue-tracker.md`. Do not require extra approval of test seams or a separate ticket-breakdown approval. Ask the user when a decision is actually needed.

## Build

Use `implement-spec` to implement the tickets on the run's feature branch, building each ticket test-first with `tdd`. Work sequentially in the run's worktree. Run the project's relevant checks and fix failures before moving on. Find the project's verification: the commands its agent instructions (`AGENTS.md`, `CLAUDE.md`), contributing docs or build files say to run before merging, which should match what its CI runs. Run all of them after each ticket, not only the relevant ones; reuse a passing result only while the checked code and its relevant inputs are unchanged, and always honor stricter checks the project requires. A missing tool or a failing check blocks the run, so fix it or mark the run blocked, and never skip it. If you cannot find any, ask the user once, or say so in the pull request when running unattended.

## Check

Before Check, commit the completed implementation: make sure the full verification has passed on exactly this code (rerun it if anything changed since it ran), then commit everything that belongs to the change, including new files but not scratch or generated files (ignore or remove those), with a normal commit (never amend), so `git status --porcelain` is empty. `code-review` diffs committed work only, so uncommitted changes would be missed. Pin the review: the baseline is the base commit recorded for the run (`--base-commit`; read it with `factory run inspect`), and the reviewed commit is `HEAD`; record the reviewed commit's short hash with `--summary check="Review pinned at <hash>"` so a new session can find it, and repeat the latest reviewed hash in the Check summary you write when the stage ends, because a new summary replaces the old one.

Use `code-review` with the base commit as the fixed point, and with the spec, then use `security-review` on the same range (base commit to `HEAD`). Tell `code-review` which model and effort its agents use, as described in Model routing in [factory-procedures](../factory-procedures/SKILL.md). When there are no actionable findings, go to Finish.

When there are actionable findings, start a Rework: return to Build, fix the findings test-first with `tdd`, run the relevant checks and then the full verification, and commit the fixes together with any new files. Then run Check again with both reviews (`code-review` and `security-review`), each using the previously reviewed commit as the fixed point so the review covers exactly the Rework's changes, together with the findings it addressed. Pin the new `HEAD` as the reviewed commit, replacing the earlier pin. A run gets at most two Reworks.

Fix actionable findings yourself within the Rework allowance; findings alone never block a run. When findings remain after the second Rework, or a finding cannot be resolved, the run needs a decision from the user, so report it as blocked: `factory run update <id> check blocked --blocking-reason "Decision needed on remaining Check findings"`. List the remaining findings and ask the user whether to do one more Rework, finish with the findings listed in the pull request, or stop. Follow their choice, and when work resumes clear the reason with `factory run update <id> <build|finish> running --clear-blocking-reason`. If they stop the run, leave it blocked.

## Finish

Use `pr` to write the pull request description. When the user chose to finish with remaining Check findings, list them in the description. The implementation and Reworks are already committed; commit anything still uncommitted that belongs to the change as a new commit. Verify the final commit before pushing: reuse the latest passing full verification when nothing has changed since it ran (same `HEAD`, clean tree), otherwise run the project's verification on it. Report the result; do not open the pull request while a check fails or cannot run. Then follow Pull request and CI in [factory-procedures](../factory-procedures/SKILL.md).
