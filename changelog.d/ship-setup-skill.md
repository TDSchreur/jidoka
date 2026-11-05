## Added

- `factory install` now also installs `setup-matt-pocock-skills`. Run `/setup-matt-pocock-skills` once in each repository to configure its issue tracker, triage labels and domain docs before using the workflows.
- `/factory`, `/factory-balanced` and `/factory-fast` stop at the start of Define when `docs/agents/issue-tracker.md` is missing and tell you to run `/setup-matt-pocock-skills`. Setup output that is not yet committed is carried from the original checkout into the run's worktree and included in its pull request.

## Changed

- `factory install` no longer installs `github-actions-hardening` and `rust-best-practices`, which are development tools for this project, and removes copies installed by earlier versions.
