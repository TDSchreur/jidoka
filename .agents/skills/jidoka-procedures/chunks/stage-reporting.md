## Run reporting

Record stage summaries and ticket status with `jidoka run update <id>`; use `--current-ticket <reference>` while working on a ticket and clear it afterward. Record skill and activity reports at milestones. When outside input is needed, use `jidoka run block <id> --reason "<what is needed>"`. Use `jidoka run resume <id> --source "<what let work resume>"` when the block is resolved. Never include secrets. For options and continuation details, consult `jidoka-procedures/chunks/run-reporting.md` in the installed skills. Leave a stage with `jidoka run advance <id> --summary <stage>="<one or two sentences>" --activity "<what the next stage does>"`; it records both and prints the next stage's instructions, so no separate `jidoka run next` or `jidoka run update` is needed.

### Codex command approvals

When a Jidoka command needs escalation in Codex and a reusable approval is appropriate, pass the narrow, stable subcommand as the tool's `prefix_rule`. For example, use `["jidoka", "pr", "wait"]` for PR monitoring. Keep run IDs, titles and paths in the command arguments so changing them does not invalidate the saved approval. Keep any flags that limit what the approved command can do. Avoid a blanket `jidoka`, `jidoka run` or `jidoka pr` prefix.

For cleanup, confirm that the user has authorized removing the merged run's worktree and local branch before requesting escalation. If the user wants a reusable cleanup approval and the host’s approval rules permit a persistent approval for cleanup, use `["jidoka", "run", "cleanup"]` and explain that saving it covers future run IDs. Otherwise request one-time approval. Jidoka checks whether the recorded pull request has merged before removing the worktree or branch.
