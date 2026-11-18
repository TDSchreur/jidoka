---
status: accepted
---

# Change groups link runs through a server-owned `groups.json`

A platform change can span several repositories and need several pull requests, while a run has one repository, worktree, branch and pull request. A **change group** links ordinary runs to one shared spec and leaves every run's stages, gates, verification and cleanup alone. Groups are advisory: they never block a run or a merge.

- **Storage.** Groups live in `groups.json` beside `runs.json`, owned by the dashboard server like runs. The CLI changes groups through `/api/groups`, as it already writes runs through the API. The group record owns membership; a run's group is derived from the groups.
- **Locking.** The server holds a group lock. An operation that needs both locks takes the group lock first, then the run lock. Validating a member (the run exists, it is in no other group) and saving happen under one group lock, so concurrent attachments cannot put a run in two groups. Deleting a run holds the group lock from its membership check through the deletion, so attachment and deletion are serialized: if attachment wins, deletion is refused; if deletion wins, attachment reports a missing run.
- **IDs.** The run ID generator with a `g` prefix, so a group ID is never mistaken for a run ID.
- **Lifecycle.** `group delete` removes only a group with no members. There is no archive in v1; add one if `group list` gets noisy.
- **Deleting a grouped run.** The server refuses with 409 and tells the user to detach it first. Inspection still reports a member whose run is missing (for example after a hand edit of `runs.json`), and `group remove` detaches it.
- **Dependencies are advisory.** Later steps add prerequisites between members (an upstream pull request merged, a named milestone confirmed). They are shown to the operator and in pull request text and never enforced.
- **Pull request identity.** A merge dependency clears only when the upstream run has a current pull request URL, the stored status URL matches it, and the stored state is merged, so a previous pull request's merge cannot satisfy its replacement. Group inspection already shows a status that belongs to another URL as unchecked.
- **Milestones.** A milestone is created once in its group, and each dependent run references its ID, so confirming or reopening it applies to every consumer. A changed requirement is a new pending milestone; an unreferenced one can be deleted. Jidoka has no user identity, so a confirmation stores its time and an optional evidence URL, not who confirmed.
- **Out of scope.** CI and review details: only the pull request state Jidoka already records is shown.
- **Limits.** A group holds at most 50 runs, a title is at most 200 bytes and a spec at most 4096 bytes, so a request body and a listing stay small. `create` rejects a run given twice.
- **Specs.** A group needs a title and a spec: a durable web link, or a local file, which the CLI stores as an absolute path at creation and the server requires to be absolute.

## Considered Options

- **Store groups inside `runs.json`.** Rejected: the only write lock is the server's in-process mutex around the run store, so a group edit would have to share it and change the `runs.json` format that every run read depends on. A separate file leaves that format alone.
- **A `group` field on each run.** Rejected: membership validation would need to read and write many runs at once, and a run could silently land in two groups.
- **Archive instead of delete.** Rejected for v1: the least to build wins until the list is noisy.
- **Deleting a grouped run detaches it.** Rejected: it hides a broken link the user should decide about.

## Consequences

- A new file in the data folder. Restart the dashboard after upgrading so it serves `/api/groups`; `jidoka run inspect` fails against an older dashboard, since it reads the groups to show a run's.
- The dashboard's delete of a grouped run now fails with a message until the run is detached.
- Nothing migrates: runs without a group behave as before.
