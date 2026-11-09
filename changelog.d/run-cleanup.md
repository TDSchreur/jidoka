## Added

- `factory run cleanup <run-id>` removes the worktree and local branch of a run whose pull request merged. It refuses, removing nothing, when the pull request is not merged, the worktree has uncommitted changes, the local branch is not at the merged head commit, or you are inside the worktree. The remote branch is left alone, and the run stays on the dashboard with its worktree and branch cleared.
- `factory run cleanup` offers to reinstall the factory (`cargo install` and `factory install`) when the run changed the factory itself and your checkout already contains the merged pull request. `--reinstall` does it without asking, and off a terminal the commands are printed.

## Changed

- The Finish report of the factory workflows tells you to run `factory run cleanup <id>` once the pull request merges, instead of `git worktree remove`.
