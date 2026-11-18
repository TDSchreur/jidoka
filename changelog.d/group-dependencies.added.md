## Added

- Add group milestones and dependencies: `jidoka group milestone add|remove`, `depend`, `undepend`, `confirm` and `reopen` let a run wait on another member's merged pull request or on a named milestone, such as a published module. `group inspect` and `jidoka run next` in Finish show the outstanding conditions, and the Finish and pull request procedures tell the agent to describe them in the pull request and link the shared spec. Dependencies are advisory and never block a run. See the new [multi-repo guide](docs/multi-repo.md). Restart the dashboard after upgrading.
