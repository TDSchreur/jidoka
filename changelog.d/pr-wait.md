## Added

- `factory pr wait <run-id>` watches the checks of a run's pull request and prints one CI outcome with its own exit status: passed, failed, cancelled, timeout, head-changed, no-ci or not-open. A failure comes with the failed checks, their links and the end of their logs. It also says whether the branch is behind its base. Options: `--timeout`, `--fail-fast` and `--json`.

## Changed

- The Finish procedure of the factory workflows watches CI with `factory pr wait` instead of polling by hand, and falls back to the manual watch when the installed `factory` does not know the command.
