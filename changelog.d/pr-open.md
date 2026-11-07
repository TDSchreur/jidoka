## Added

- `factory pr open <run-id> --title <title> --body-file <path>` pushes a run's branch and opens its pull request, or finds the one that is open for the branch, and records the URL on the run. It checks first that the worktree is clean and on the recorded branch, that the branch is not the base and has commits beyond it, and that `gh` is signed in; it never forces a push, and running it again never makes a second pull request. Options: `--base` and `--json`.

## Changed

- The Pull request and CI procedure of the factory workflows opens the pull request with `factory pr open`, and falls back to doing it by hand when the installed `factory` does not know the command.
