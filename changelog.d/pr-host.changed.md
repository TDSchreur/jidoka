## Changed

- Problems with `gh` and `az` read the same in every command that talks to GitHub or Azure Repos (`jidoka pr open`, `pr wait`, `run sync`, `run cleanup` and the dashboard's pull request state): one message each for a missing tool, a missing sign-in and a network problem, and an Azure CLI failure names the operation that failed and never shows Azure's own output.
