---
name: factory-balanced
description: Take a software change from an idea to a pull request through define, build, check, and finish, with lightweight planning, tests after implementation, and one review pass. Use when the user asks to run factory-balanced.
---

# TDS Factory Balanced

Take the user's request through these stages in order. Keep stage updates brief. Load only the skills needed for the task, reading their `SKILL.md` files and relevant referenced resources. In this project's checkout, bundled implementation and planning skills are in `skills/<name>/`; in an installed bundle, they are siblings of this directory. Otherwise use the corresponding installed skills. This workflow's instructions take precedence where they differ from those skills. Read [factory-procedures](../factory-procedures/SKILL.md) before starting: it holds the procedures shared by all three workflows (run reporting, continuing a run, workspace, tracker setup, pull request and CI).

## Run reporting

Report progress to the factory dashboard as described under Run reporting in [factory-procedures](../factory-procedures/SKILL.md), with `--mode factory-balanced`. This workflow has the stages define, build, check and finish.

When Check sends the run back to Build, report it as a Rework: `factory run update <id> build running --rework <n> --activity "Rework <n>: <findings being fixed>"`, numbering Reworks from 1 within the run and leaving the count set when Check runs again.

## Continuing a run

To continue an existing run, follow Continuing a run in [factory-procedures](../factory-procedures/SKILL.md).

## Workspace

Follow Workspace in [factory-procedures](../factory-procedures/SKILL.md): every run gets its own fresh worktree, and the PR base branch chosen there is used in Finish.

## Define

When the request is clear and bounded, use it as the spec and state the plan and explicit acceptance criteria in the conversation; no tracker setup is needed for that. Otherwise follow Tracker setup in [factory-procedures](../factory-procedures/SKILL.md) and use `to-spec` once to write a concise spec from the request and the code, without interviewing the user, stating explicit acceptance criteria either way. Before Build, if the plan and criteria exist only in the conversation, follow Saving the plan in [factory-procedures](../factory-procedures/SKILL.md).

Use `to-tickets` only when the work has multiple pieces that benefit from separate tracking. Use `grill-with-docs` only when an ambiguity blocks implementation, and limit it to that ambiguity. Reuse existing specs and tickets rather than recreating them. Reading or publishing specs and tickets in the tracker needs Tracker setup first.

When these artifacts are needed, store them in the tracker configured in `docs/agents/issue-tracker.md`. Do not require extra approval of test seams or a separate ticket-breakdown approval. Ask the user when a decision is actually needed.

## Build

Use `implement-spec-balanced` to implement the request or tickets on the run's feature branch: implement each ticket, then add behavior tests through public interfaces without red-green cycles or seam approval. Work sequentially in the run's worktree; do not require subagents, extra worktrees beyond it, a draft PR, or a code review here. Run the project's relevant checks for each ticket and fix failures before moving on. Find the project's verification: the commands its agent instructions (`AGENTS.md`, `CLAUDE.md`), contributing docs or build files say to run before merging, which should match what its CI runs. Run all of them once, after the last ticket and on the candidate you will commit before Check, not after every ticket; reuse a passing result only while the checked code and its relevant inputs are unchanged, and always honor stricter checks the project requires. A missing tool or a failing check blocks the run, so fix it or mark the run blocked, and never skip it. If you cannot find any, ask the user once, or say so in the pull request when running unattended.

## Check

Before Check, commit the completed implementation: make sure the full verification has passed on exactly this code (rerun it if anything changed since it ran), then commit everything that belongs to the change, including new files but not scratch or generated files (ignore or remove those), with a normal commit (never amend), so `git status --porcelain` is empty. `code-review` diffs committed work only, so uncommitted changes would be missed. Pin the review: the baseline is the base commit recorded for the run (`--base-commit`; read it with `factory run inspect`), and the reviewed commit is `HEAD`; record the reviewed commit's short hash with `--summary check="Review pinned at <hash>"` so a new session can find it, and repeat the latest reviewed hash in the Check summary you write when the stage ends, because a new summary replaces the old one.

Use one `code-review` pass with the base commit as the fixed point, and with the spec or acceptance criteria (the recorded spec or saved plan). Use `security-review` on the same range only when the diff touches input parsing, authentication or secrets, network access, file or process handling, or dependencies; otherwise skip it and say why.

When there are no actionable findings, go to Finish. When there are, start a Rework: return to Build, fix the findings with `implement-spec-balanced` (implement, then add or adjust behavior tests), run the relevant checks and then the full verification on the changed code, and commit the fixes together with any new files. Then run Check again with one `code-review` pass using the previously reviewed commit as the fixed point, so the review covers exactly the Rework's changes, together with the findings it addressed, plus `security-review` over the same range when those changes meet the criteria above. Pin the new `HEAD` as the reviewed commit, replacing the earlier pin. A run gets at most two Reworks.

Never block work on Check findings. When findings remain after the second Rework, or a finding cannot be resolved, keep the run running with `--activity "Waiting for a decision on remaining Check findings"`, list the remaining findings, and ask the user whether to do one more Rework, finish with the findings listed in the pull request, or stop. Follow their choice.

## Finish

Use `pr` to write the pull request description. When the user chose to finish with remaining Check findings, list them in the description. The implementation and Reworks are already committed; commit anything still uncommitted that belongs to the change as a new commit. Verify the final commit before pushing: reuse the latest passing full verification when nothing has changed since it ran (same `HEAD`, clean tree), otherwise run the project's verification on it. Report the result; do not open the pull request while a check fails or cannot run. Then follow Pull request and CI in [factory-procedures](../factory-procedures/SKILL.md).
