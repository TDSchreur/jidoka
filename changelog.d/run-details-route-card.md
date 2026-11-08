## Changed

- The run details sheet is now a route card: a header with the status, pull request and spec, then Define, Build, Check and Finish with their summaries, skills, activity reports, tickets and fresh tokens. A blocked run shows its reason in an amber status row, and token usage is a section that is open while the run is active. The worktree path and base commit are no longer shown (`factory run inspect` still prints them), and times use one short format.
