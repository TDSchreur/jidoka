# Full: Build

When Build is routed (see Model routing when supplied), hand each ticket, and a Rework's fixes, to a fresh agent on the build model as it describes; this takes precedence over the build skill's "do not require subagents".

Use `jidoka-implement` to implement the tickets on the run's feature branch. Use test-first development when allowed by project instructions; otherwise follow the project's test approach. Work sequentially in the run's worktree. Use targeted or quick checks during iteration, following the project's requirements; follow Verification in the shared procedures included below.

For each listed ticket, start work with `jidoka run update <id> --ticket-status <reference>=in-progress`. Commit the work, run the project's required ticket verification on that commit, then record `jidoka run gate <id> tickets-verified pass` while the ticket is still in progress. Record a failed check with `fail`, fix it and verify again before marking the ticket `done`. Jidoka records which ticket attempt and commit the verification covered. When the work was declared unticketed, commit and verify the Build candidate and record the same gate before leaving Build. Use `tickets-verified fail` if unticketed verification fails, then fix and verify again.

For a Rework, reopen only the listed tickets whose work the fixes revisit. Mark each affected ticket in progress and follow the same commit and verification steps for its new attempt. Unaffected tickets keep their evidence. For a fix-only Rework that leaves the listed tickets' work intact, keep their statuses and verify the reviewed candidate as directed in Check. If the run is unticketed, record verification on the Rework's Build candidate before advancing.
