## Added

- `/factory` and `/factory-balanced` send actionable Check findings back to Build for a Rework, then run Check again, with at most two Reworks per run. When findings remain after that, or one cannot be resolved, the agent asks the user whether to do one more Rework, finish with the findings listed in the pull request, or stop, instead of blocking the run. `/factory-fast` is unchanged.
- `factory run update` accepts `--rework <n>` and `--clear-rework`, the API accepts a `rework` count, and the dashboard shows it as "Rework 1 of 2". Existing run records load without a count.
