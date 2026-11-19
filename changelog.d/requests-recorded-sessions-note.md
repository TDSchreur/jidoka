## Fixed

- `jidoka run requests` no longer says no session was recorded when the run's sessions were recorded but their logs are in a Claude folder it did not scan. The note now names the scanned folders and points to `JIDOKA_CLAUDE_CONFIG_DIRS`, and `--all` gives the same reason for the runs it leaves out.
