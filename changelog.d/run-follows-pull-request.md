## Added

- `factory run sync <run-id> [--json]` and a "Check PR" button on the dashboard's run sheet read the run's pull request from its host and bring the run in line with it.

## Fixed

- A run no longer stays on "Waiting for CI" after its pull request is merged or closed. The dashboard, `factory run sync` and `factory pr wait` clear the activity and mark Finish done for a merged pull request, or block the run with "PR closed without merge" for a closed one.
