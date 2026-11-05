# Project-agnostic bundled skills

## Problem and outcome

The skills that `factory install` ships are used by anyone on any kind of project. PR #50 and #51 put this repository's own `scripts/verify.sh` into the workflow skills and the two local `implement-spec-*` skills, and the Finish stage of all three workflow skills says "(`develop` for this project)". For every other project the verify rule never triggers and the branch hint is wrong. Make the shipped text generic, keep the repo-specific answers in this repo's own instructions, and add a guard so it cannot regress.

## Behavior

- The workflow skills (`factory`, `factory-balanced`, `factory-fast`) and `implement-spec-balanced` / `implement-spec-fast` tell the agent to find the project's verification: the commands its agent instructions (`AGENTS.md`, `CLAUDE.md`), contributing docs or build files say to run before merging, which should match what its CI runs. All of them run after each ticket (`factory`, `factory-balanced`) or before Finish (`factory-fast`) and on the final commit before Finish. A missing tool or a failing check blocks the run: fix it or mark the run blocked, never skip. If no verification can be found, ask the user once, or when unattended say so in the pull request. The CI-watch paragraph re-runs the project's verification instead of naming a script.
- The Finish stage targets the base branch chosen in Workspace instead of naming `develop`.
- `scripts/verify.sh` stays this repository's answer, referenced only from `AGENTS.md` and the README. The upstream `skills/implement-spec` is untouched.
- A test fails when any bundled skill contains this repository's names (`verify.sh`, `tds-factory`, `for this project`), or when a workflow skill contains `develop` as a word. The list lives in one place with a comment saying why.
- `AGENTS.md` and `.github/instructions/agent-skills.instructions.md` state the rule, so every agent that reads them follows it; the instruction's `applyTo` also covers `.agents/skills/**/SKILL.md`.

## Verification

The guard test is written first and fails on the current skills; after the rewrite it passes. `scripts/verify.sh` passes. The upstream skills were searched for the forbidden terms beforehand and contain none, so no exemptions are needed.

## Tickets

1. Guard test in `src/install.rs`, red on the current skills.
2. Generic wording in the five skills and the base-branch reference. Makes ticket 1 green.
3. Repo rule in `AGENTS.md` and `agent-skills.instructions.md`, changelog fragment, README only if a claim is wrong. Independent of 2.
