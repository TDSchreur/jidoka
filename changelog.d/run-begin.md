## Added

- `factory run begin "<title>"` creates a run and its workspace in one command: it fetches the base branch, creates a worktree on a new branch beside the main checkout, records mode, worktree, branch and base commit, and prints them (`--json` for data). Options: `--agent`, `--mode`, `--base`, `--branch`. A name that is taken gets a suffix, and `factory run begin --run <id>` resumes a run whose workspace was only partly made. It never deletes, resets or cleans anything, and a failure marks the run blocked with the reason and the retry command.

## Changed

- The Workspace and Run reporting procedures of the factory workflows create the run and its workspace with `factory run begin`, and fall back to the manual steps when the installed `factory` does not know it.
