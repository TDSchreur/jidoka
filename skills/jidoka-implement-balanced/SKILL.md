---
name: jidoka-implement-balanced
description: Implement a spec and its tickets with project-directed behavior tests, without red-green cycles. Use for the Build stage of jidoka-balanced or when tested implementation without strict TDD is requested.
---

# Implement Spec Balanced

Implement the provided spec on a single feature branch. A clear user request with acceptance criteria can serve as the spec; tickets are optional. Use existing tickets when provided, without requiring new planning artifacts or tracker setup.

1. Read the request or spec, its acceptance criteria, and any tickets, identify dependencies, and inspect the relevant code and project instructions. Reuse context already gathered by the calling workflow.
2. Create or use the task's feature branch. Work sequentially in the current checkout, taking tickets whose dependencies are satisfied or implementing the agreed plan directly when there are no tickets.
3. Implement each ticket first, without invoking `jidoka-tdd` or running a red-green-refactor cycle.
4. Then add or update behavior tests for the ticket’s acceptance criteria when appropriate under the project’s instructions. Apply the test-quality rules from `jidoka-tdd` (its "What a good test is" and "Anti-patterns" sections, plus [tests.md](../jidoka-tdd/tests.md) and [mocking.md](../jidoka-tdd/mocking.md) when needed):
   - test through public interfaces, preferring existing seams;
   - take expected values from an independent source such as the spec or a worked example, never by recomputing them the way the code does;
   - do not mock internal collaborators, test private functions, or verify through a side channel.

   Choose the seams yourself; do not ask for seam approval. Follow the project’s instructions on test scope and explain any omission.
5. Run targeted or quick checks during iteration and fix failures, following the project's required cadence. Reuse passing results only while the checked code and its relevant inputs are unchanged; rerun affected checks after changes and honor stricter checks the project requires. Run the project's full verification at the point the calling workflow sets, or after the last ticket when used on its own; a missing tool or a failing check is a blocker, never something to skip.
6. When the change has a runnable entrypoint (application, server, CLI), start it once from the working directory a user would use, with its default configuration and no test overrides, and exercise the main path; report what you ran, or why no entrypoint applies.
7. Verify the result against the acceptance criteria and end by listing each criterion with the test or evidence that shows it. Report the completed work, tests added, checks performed, and any remaining blockers. Leave commits, pushing, and PR creation to the calling workflow unless the user explicitly requests them.

Do not require subagents, extra worktrees, a draft PR, or a separate code review as part of this skill.
