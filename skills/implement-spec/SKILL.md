---
name: implement-spec
description: "Implement the result of /to-spec and /to-tickets in code, test-first with tdd."
disable-model-invocation: true
---

# Implement Spec

Adapted from the `implement-spec` skill in mattpocock/skills (MIT); see the license installed with this skill.

Implement the provided spec and its tickets on a single feature branch, building each ticket test-first. The tickets are a **task graph** with blocking relationships, not a list of steps, so always take a ticket whose blockers are done.

The issue tracker should have been provided to you. If not, tell the user to run `/setup-matt-pocock-skills`.

1. Read the spec and tickets, identify dependencies, and inspect the relevant code and project instructions. Reuse context already gathered by the calling workflow.
2. Create or use the task's feature branch. Work sequentially in the current checkout, one ticket at a time.
3. Build each ticket by calling the Skill tool with `tdd`.
4. After each ticket, run the project's relevant checks and then all of its verification commands (those its agent instructions, contributing docs or build files say to run before merging, which should match its CI), reusing a passing result only while the checked code and its relevant inputs are unchanged and always honoring stricter checks the project requires; a missing tool or a failing check is a blocker, never something to skip. If you cannot find any verification, ask the user once, or say so in your report when running unattended. Fix failures before the next ticket.
5. Resolve each finished ticket the way the issue tracker closes work, unless the calling workflow handles that.
6. Verify the result against the spec. Report the completed work, tests added, checks performed, and any remaining blockers. Leave commits, pushing, and PR creation to the calling workflow unless the user explicitly requests them.

Do not require subagents, extra worktrees, a draft PR, or a separate code review as part of this skill.
