## Added

- The Done column on the dashboard shows each Run on one line, with a Show details toggle for the rest. Each Done Run has a Delete button, and the column has Delete all; both ask for confirmation first. Deleting removes only the Run record: the worktree, branch and pull request stay as they are.
- `DELETE /api/runs/<id>` removes a Run record. Like the other write requests it needs a local host and refuses a foreign origin, and it also needs the session token that the dashboard page carries. The token changes each time the dashboard server starts, so restart the dashboard after upgrading and reload the page.
