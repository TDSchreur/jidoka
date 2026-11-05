## Added

- A run records the context a new session needs to continue it: its canonical repository (the same from every linked worktree), worktree, branch and base commit, workflow mode, spec (a URL or local path), ordered tickets with progress, pull request, a short summary per Stage and the status reason. It is recorded through new `factory run update` options (`--mode`, `--worktree`, `--branch`, `--base-commit`, `--spec`, `--pr`, `--ticket`, `--ticket-status`, `--summary` and `--clear-*` forms) and the API; omitted fields are kept, inconsistent updates are refused with a reason, and records from before it show the missing context as unknown.
- `factory run inspect <id> [--json]` prints a run's context, backed by a new `GET /api/runs/<id>`.
- The dashboard groups the runs of linked worktrees under their repository, shows the context in the run detail, and shows ticket progress and pull request and spec links on cards. `factory dashboard` opens on the current repository.
- The three workflow skills record this context at milestones and, when asked to continue a run, verify the recorded repository, worktree, branch, base commit and Git state first, report differences, and never reset, clean or discard work.

## Changed

- The run detail calls the blocking reason the "Status reason".
