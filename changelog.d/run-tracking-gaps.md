## Changed

- Every `factory run update` re-collects the run's token usage, including repeated updates and every `finish done`. Collection replaces the totals, so nothing is counted twice.
- `factory run update <id> finish done` is refused unless the run has a non-empty Finish summary, supplied with `--summary finish=<what shipped>` or recorded earlier. The run stays unchanged and the message names the flag.
- The run details place each report in the last pass of the stage it was reported with that had started by then, or the nearest pass of that stage, instead of by time. Reports without a stage (older runs) still go by time.

## Added

- Runs record when they were blocked and resumed (`events` in `factory run inspect` and the API), and the run details show a "Blocked: <reason>" or "Resumed" line in the pass that holds it. Events do not change the timeline or token usage.
