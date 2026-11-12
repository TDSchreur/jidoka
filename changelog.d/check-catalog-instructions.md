## Changed

- Deliver applicable Check instructions and model routing from the Check catalog, and report unavailable diffs instead of silently skipping triggered checks.

- Breaking: record each applicable Check separately with `jidoka run gate <id> checks-recorded pass --check <name>`; results apply to the recorded commit and preserve skipped-check snapshots.
