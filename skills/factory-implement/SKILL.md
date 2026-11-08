---
name: factory-implement
description: "Implement the result of /factory-to-spec and /factory-to-tickets in code, test-first with factory-tdd."
disable-model-invocation: true
---

# Implement Spec

Adapted from the `factory-implement` skill in mattpocock/skills (MIT); see the license installed with this skill.

Implement the provided spec and its tickets on a single feature branch, building each ticket test-first. The tickets are a **task graph** with blocking relationships, not a list of steps, so always take a ticket whose blockers are done.

The issue tracker should have been provided to you. If not, tell the user to run `/factory-setup`.

1. Read the spec and tickets, identify dependencies, and inspect the relevant code and project instructions. Reuse context already gathered by the calling workflow.
2. Create or use the task's feature branch. Work sequentially in the current checkout, one ticket at a time.
3. Build each ticket by calling the Skill tool with `factory-tdd`.
4. After each ticket, run the project's relevant checks and fix failures before the next ticket. Run the project's full verification at the point the calling workflow sets, or after the last ticket when used on its own; a missing tool or a failing check is a blocker, never something to skip. Find verification in the project's agent instructions, contributing docs or build files; if none is found, ask the user once, or report it when unattended. Reuse a passing result only while the checked code and relevant inputs are unchanged, and honor stricter project requirements.
5. Resolve each finished ticket the way the issue tracker closes work, unless the calling workflow handles that.
6. Verify the result against the spec. Report the completed work, tests added, checks performed, and any remaining blockers. Leave commits, pushing, and PR creation to the calling workflow unless the user explicitly requests them.

Do not require subagents, extra worktrees, a draft PR, or a separate code review as part of this skill.
