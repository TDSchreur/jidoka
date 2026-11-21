# Record run completion and pull request state

Done means Jidoka ended work on a run. Jidoka records the Finish summary and how the run completed. A pull request has its own state because the run can finish with CI confirmed while the pull request is open. An observed merge can also finish a run without CI evidence. A forced completion carries the explicit reason.

The run record and API responses expose a server-owned `completion` object. Its `kind` is `ci` with the confirmed `head` and an `outcome` of `passed` or `no-ci`, `merged` with the observed pull request `url`, or `override` with a `reason`. A later merge updates `pr_status` and keeps the completion evidence. Older records load without a completion reason.

Callers that used `jidoka run update <id> done` or posted `status: "done"` to the reporting endpoint must use `jidoka run advance` or the advance endpoint to complete Finish. The Finish summary can be recorded before advancing. Merge reconciliation supplies a summary if needed.
