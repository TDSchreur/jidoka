## Changed

- Problems with `gh` and `az` read the same in every command that talks to GitHub or Azure Repos (`jidoka pr open`, `pr wait`, `run sync`, `run cleanup` and the dashboard's pull request state): one message each for a missing tool, a missing sign-in and a network problem, and an Azure CLI failure names the operation that failed and never shows Azure's own output.
- `jidoka pr wait` on an Azure Repos pull request now also gives the pull request up to 30 seconds to show a commit that was just pushed, as it already did on GitHub.
- `jidoka pr open` and `jidoka pr sync` check the run's Workspace the same way: `pr open` now also says when a merge is in progress in the worktree, and `pr sync` names the worktree when it is on no branch or cannot be read, and says that a pull request needs a feature branch when it is on the base branch.
