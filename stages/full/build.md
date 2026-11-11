# Full: Build

When Build is routed (see Model routing when supplied), hand each ticket, and a Rework's fixes, to a fresh agent on the build model as it describes; this takes precedence over the build skill's "do not require subagents".

Use `jidoka-implement` to implement the tickets on the run's feature branch. Use test-first development when allowed by project instructions; otherwise follow the project's test approach. Work sequentially in the run's worktree. Use targeted or quick checks during iteration, following the project's requirements; follow Verification in the shared procedures included below.

For each ticket implemented in Build, set `--current-ticket <reference>` with `jidoka run update <id> running`, including local ticket references. After implementing the ticket, run the project's required verification and record `jidoka run gate <id> tickets-verified pass` before marking the ticket done or starting the next one. Record a failure with `fail` and fix it before continuing. The gate uses the current ticket reference. During a fix-only Rework, verify the fixes as directed in Check; previously verified tickets need not be repeated.
