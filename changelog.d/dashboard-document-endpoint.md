## Added

- The dashboard serves a run's recorded local spec or ticket as safe, rendered Markdown at `GET /api/runs/<id>/document?reference=<reference>`: only recorded `.md` files up to 1 MB inside the run's worktree, repository or saved plan, with raw HTML escaped and only http(s) links kept.
