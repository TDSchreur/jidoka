## Added

- `factory run check <run-id>` compares what a run records with the repository before a session continues it: the repository and its git directory, the worktree, the branch, the base commit and the working tree. It reports each as matching, differing or unknown, lists commits that are not pushed, exits with status 2 when something differs, and never changes anything. The workflows run it when they continue a run and fall back to the manual checks when the command is unknown.
