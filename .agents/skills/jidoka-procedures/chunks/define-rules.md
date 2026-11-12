## Define rules

Do not require extra approval of test seams or a separate ticket-breakdown approval. Ask the user when a decision is actually needed.


## Saving the plan

When a run's plan and acceptance criteria exist only in the conversation, save them before Build so another session can continue without the original chat. Skip this when the spec is already a tracker issue or a file in the repository, and record that reference instead.

1. Take the plan path from `jidoka run begin` or `jidoka run inspect` (the `Plan` line, or `plan` in `--json`), and write a concise Markdown file with the task, the plan and the acceptance criteria there. It lives in the Jidoka data folder, outside the repository and the pull request, and stays after the worktree is cleaned up.
2. Record it with `jidoka run update <id> <status> --spec <absolute path>`. This is the same context option as a tracker spec, so no tracker is involved.
3. When the plan changes, rewrite the file. Never save secrets or a transcript.

