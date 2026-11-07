## Changed

- The factory workflows now keep the pull request current with its base branch. Before pushing and whenever a CI watch ends, the agent checks whether another pull request has left it behind. If so, it merges the base branch into the branch with a normal commit, never a rebase or force-push, resolves and verifies first when the merge conflicts or touches the same files, and watches CI again. A pull request is reported ready only when it is up to date and CI is green.
- CI caches compiled dependencies with `Swatinem/rust-cache`, pinned to a commit. The cache is keyed on the toolchain and `Cargo.lock`, and the checks still run in full with `--locked`.
