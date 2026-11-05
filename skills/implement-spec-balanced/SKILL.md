---
name: implement-spec-balanced
description: Implement a spec and its tickets, then cover each with behavior tests, without red-green cycles. Use for the Build stage of factory-balanced or when tested implementation without strict TDD is requested.
---

# Implement Spec Balanced

Implement the provided spec on a single feature branch. A clear user request with acceptance criteria can serve as the spec; tickets are optional. Use existing tickets when provided, without requiring new planning artifacts or tracker setup.

1. Read the request or spec, its acceptance criteria, and any tickets, identify dependencies, and inspect the relevant code and project instructions. Reuse context already gathered by the calling workflow.
2. Create or use the task's feature branch. Work sequentially in the current checkout, taking tickets whose dependencies are satisfied or implementing the agreed plan directly when there are no tickets.
3. Implement each ticket first, without invoking `tdd` or running a red-green-refactor cycle.
4. Then add or update behavior tests for the ticket's acceptance criteria. Apply the test-quality rules from `tdd` (its "What a good test is" and "Anti-patterns" sections, plus [tests.md](../tdd/tests.md) and [mocking.md](../tdd/mocking.md) when needed):
   - test through public interfaces, preferring existing seams;
   - take expected values from an independent source such as the spec or a worked example, never by recomputing them the way the code does;
   - do not mock internal collaborators, test private functions, or verify through a side channel.

   Choose the seams yourself; do not ask for seam approval. Skip tests only when the change has no testable behavior, such as documentation, and say so.
5. Run the project's relevant checks after each ticket and fix failures before the next one. Reuse passing results for unchanged code; rerun affected checks after changes and honor project-required checks. Run the project's full verification (the commands its agent instructions, contributing docs or build files say to run before merging, which should match its CI); a missing tool or a failing check is a blocker, never something to skip.
6. Verify the result against the acceptance criteria. Report the completed work, tests added, checks performed, and any remaining blockers. Leave commits, pushing, and PR creation to the calling workflow unless the user explicitly requests them.

Do not require subagents, extra worktrees, a draft PR, or a separate code review as part of this skill.
