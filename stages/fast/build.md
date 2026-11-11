# Fast: Build

When Build is routed (see Model routing when supplied), hand each ticket to a fresh agent on the build model as it describes; this takes precedence over the build skill's "do not require subagents".

Use `jidoka-implement-fast` to implement the request or tickets on the run's feature branch without TDD. Work sequentially in the run's worktree; do not require other subagents, extra worktrees beyond it, a draft PR, or a code review here. Use targeted or quick checks during iteration, following the project's requirements; follow Verification in the shared procedures included below.

Before leaving Build, commit the final candidate with a clean worktree, run the project's full verification on that commit, and record it with `jidoka run gate <id> verified pass`. Finish can reuse this result while the checked code and relevant inputs remain unchanged.
