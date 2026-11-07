## Added

- `factory pr sync <run-id>` brings a run's branch up to date with its base with a normal merge commit and reports one outcome with its own exit status: current, pushed (merged and pushed, watch CI again), needs-verification (merged, but the base touched files of the change, so it is not pushed) or conflict (left in progress with the files listed). It never rebases, amends, force-pushes, resets or aborts, never pushes commits that are not its own merge, and running it again pushes a merge whose push failed. Options: `--base` and `--json`.

## Changed

- The Pull request and CI procedure of the factory workflows keeps the pull request current with `factory pr sync`, and falls back to the manual steps when the installed `factory` does not know the command.
