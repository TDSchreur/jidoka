---
name: implement-spec-fast
description: Implement a spec and its tickets without TDD. Use for the Build stage of factory-fast or when implementation without TDD is requested.
---

# Implement Spec Fast

Implement the provided spec on a single feature branch. A clear user request with acceptance criteria can serve as the spec; tickets are optional. Use existing tickets when provided, without requiring new planning artifacts or tracker setup.

1. Read the request or spec and any tickets, identify dependencies, and inspect the relevant code and project instructions. Reuse context already gathered by the calling workflow.
2. Create or use the task's feature branch. Work sequentially in the current checkout, taking tickets whose dependencies are satisfied or implementing the agreed plan directly when there are no tickets.
3. Implement each ticket directly without invoking `tdd` or requiring a red-green-refactor cycle. Add or update tests when they provide useful coverage of the changed behavior; tests need not precede implementation.
4. Run the project's relevant checks, fix failures, and verify the result against the acceptance criteria. Reuse passing results for unchanged code; rerun affected checks after changes and honor project-required checks. Run the project's full verification (the commands its agent instructions, contributing docs or build files say to run before merging, which should match its CI); a missing tool or a failing check is a blocker, never something to skip.
5. Report the completed work, checks performed, and any remaining blockers. Leave commits, pushing, and PR creation to the calling workflow unless the user explicitly requests them.

Do not require subagents, extra worktrees, a draft PR, or a separate code review as part of this skill.
