# Record run completion and pull request state

Done means Jidoka ended work on a run. Jidoka records the Finish summary and how the run completed. A pull request has its own state because the run can finish with CI confirmed while the pull request is open. An observed merge can also finish a run without CI evidence. A forced completion carries the explicit reason.

The run record and API responses expose a server-owned `completion` object. Its `kind` is `ci` with the confirmed `head` and an `outcome` of `passed` or `no-ci`, `merged` with the observed pull request `url`, or `override` with a `reason`. A later merge updates `pr_status` and keeps the completion evidence. Older records load without a completion reason.

Callers that used `jidoka run update <id> done` or posted `status: "done"` to the reporting endpoint must use `jidoka run advance` or the advance endpoint to complete Finish. The Finish summary can be recorded before advancing. Merge reconciliation supplies a summary if needed.

## Lifecycle requests

Ordinary reporting preserves the status, stage, block reason and Rework number. The workflow mode can be initialized once, including on an older run that has no mode. Dedicated block and resume commands validate the source state and record the reason or what resumed work. Log pauses and PR observations share those transition rules. A successful workspace retry resumes a workspace failure when it records the worktree.

A Rework is a separate Check-to-Build transition that names an open finding. The server derives the count and records the set of open findings and the agent's classification. [ADR-0006](0006-reworks-are-capped-and-the-user-decides-past-the-cap.md) governs when a recorded user decision is required. A rejected transition leaves the run record intact.

Scripts must omit the positional status from `run update` and replace status and Rework updates with the [dedicated lifecycle commands or API requests](../runs.md#reporting-runs-manually). Restart the dashboard with the updated executable and refresh the installed skills when upgrading. Existing records retain the last saved mode, status and count. Newly recorded lifecycle changes add a block source, source and reason on events, and Rework history as applicable.
