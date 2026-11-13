## Added

- Check findings are recorded as Findings with `jidoka run update <id> running --finding <check> <location|-> "<text>"`, and their outcomes (open, fixed, dismissed, deferred) with `--finding-status <id>=<state>[:<reason>]`. `jidoka run inspect` and `--json` list them, the `no-actionable-findings` gate refuses while a Finding is open, and the Check, Rework and Finish procedures record them and name deferred Findings in the pull request.

## Changed

- Breaking: `--rework <n>` from Check is refused unless the same update records at least one new `--finding`. There is no migration; the Rework instructions in `jidoka run next` show the new syntax.
