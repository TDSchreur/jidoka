## Run reporting

Record stage summaries and ticket status with `jidoka run update <id> running`; use `--current-ticket <reference>` while working on a ticket and clear it afterward. Record skill and activity reports at milestones. Mark a run blocked with `--blocking-reason` when outside input is needed. Never include secrets. For options and continuation details, consult `jidoka-procedures/chunks/run-reporting.md` in the installed skills. Leave a stage with `jidoka run advance <id> --summary <stage>="<one or two sentences>" --activity "<what the next stage does>"`; it records both and prints the next stage's instructions, so no separate `jidoka run next` or `jidoka run update` is needed.

### Codex command approvals

For every escalated Jidoka command eligible for reusable Codex approval, set the execution tool’s `prefix_rule` explicitly. Use the complete command verb as separate tokens, including any nested subcommands.

- `jidoka run inspect <id> --json` uses `["jidoka", "run", "inspect"]`.
- `jidoka run check <id>` uses `["jidoka", "run", "check"]`.
- `jidoka run advance <id> …` uses `["jidoka", "run", "advance"]`.
- `jidoka pr wait <id>` uses `["jidoka", "pr", "wait"]`.

Keep changing operands such as IDs, titles, paths and output formats in the command arguments. A saved approval should apply across runs of the same verb. Preserve restrictions required for the approved action and use one-time approval when the host or command requires it. Avoid broad prefixes such as `jidoka`, `jidoka run` or `jidoka pr`.

For cleanup, confirm that the user has authorized removing the merged run's worktree and local branch before requesting escalation. If the user wants a reusable cleanup approval and the host’s approval rules permit a persistent approval for cleanup, use `["jidoka", "run", "cleanup"]` and explain that saving it covers future run IDs. Otherwise request one-time approval. Jidoka checks whether the recorded pull request has merged before removing the worktree or branch.
