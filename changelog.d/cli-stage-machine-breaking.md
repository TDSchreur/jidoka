## Changed

- **Breaking:** a run's stage changes only through `jidoka run advance` (and the Rework path); `jidoka run update` no longer changes the stage, and neither does an API update. Use `jidoka run next` for the current Stage's instructions and open gates, `jidoka run gate` to record results, and `jidoka run advance <id> --force --reason "<text>"` for a stuck run.
- **Breaking:** the run storage format changed to hold gate results. In-flight runs are not migrated; finish or abandon them before upgrading.
- The workflow, run and model routing guides describe the CLI-driven stage machine.
