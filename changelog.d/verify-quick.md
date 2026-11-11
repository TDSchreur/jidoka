## Added

- `scripts/verify.sh --quick` for faster local iteration: it skips the Windows clippy step and runs the dashboard browser tests only when the dashboard or its tests changed. Without `--quick` it still runs every check CI runs.
