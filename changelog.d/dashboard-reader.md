## Added

- The dashboard serves a run's recorded local spec or ticket as safe, rendered Markdown at `GET /api/runs/<id>/document?reference=<reference>`: only recorded `.md` files up to 1 MB inside the run's worktree, repository or saved plan, with raw HTML escaped and only http(s) links kept.
- The dashboard has a Reader: a run's local spec or ticket (a reference that is not an http(s) link) gets a View link in the run row and the details sheet, and a local ticket's title opens it. The Reader shows the rendered file in a dialog over the sheet, says when the file is no longer available, and closes with Close or Escape without closing the sheet.
