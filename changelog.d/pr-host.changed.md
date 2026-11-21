## Changed

- Problems with `gh` and `az` read the same in every command that talks to GitHub or Azure Repos (`jidoka pr open`, `pr wait`, `run sync`, `run cleanup` and the dashboard's pull request state): one message each for a missing tool, a missing sign-in and a network problem, and an Azure CLI failure names the operation that failed and never shows Azure's own output.
- `jidoka pr wait` on an Azure Repos pull request now also gives the pull request up to 30 seconds to show a commit that was just pushed, as it already did on GitHub.
