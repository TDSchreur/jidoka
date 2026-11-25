## Fixed

- `jidoka pr wait` counts Azure DevOps PR-wide statuses when pipeline metadata confirms they belong to the current pull request head. Statuses without that evidence remain pending.
