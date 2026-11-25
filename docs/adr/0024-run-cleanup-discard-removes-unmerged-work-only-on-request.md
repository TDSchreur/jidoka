---
status: accepted
---

# `run cleanup --discard` removes unmerged work only on request

`jidoka run cleanup` proves a pull request merged before it deletes a worktree, so a run that never got a pull request, or whose pull request was closed, could only be removed by hand with git. We added `jidoka run cleanup <run-id> --discard` for those runs. Plain `cleanup` is unchanged: it still needs a merged pull request.

`--discard` takes the place of the merge proof with a check of the work itself:

- A pull request that is still open (or a draft, or unreadable) refuses, because the discard would leave it pointing at work that is gone. A closed or merged pull request, or none, is allowed.
- Uncommitted changes and commits that no remote-tracking branch has (as of the last fetch) are listed and refuse. `--force`, valid only with `--discard`, removes them anyway and passes `--force` to `git worktree remove`.
- The refusals that do not depend on merge state stay: a locked worktree, a current directory inside the worktree, a branch checked out in another worktree, relative paths, an invalid branch name, a worktree on another branch.
- The branch is deleted with `git update-ref -d`, pinned to the commit that was checked. The remote branch is never touched.
- The run record stays with its worktree and branch cleared and the activity "Discarded worktree and branch", like a normal cleanup. Deleting the record stays a dashboard action, so a grouped run keeps its place.

## Considered options

- **List and remove with `--discard` alone.** Simpler, but one flag would be enough to destroy unpushed work. Two deliberate steps keep the usual abandoned run (nothing unpushed) a single command and make losing work an explicit choice.
- **Dry-run first, then `--yes`.** Needs two invocations for every discard, also the safe ones.
- **Delete the run record as well.** Loses the history and breaks a change group; the dashboard already deletes records.
- **Allow it with an open pull request.** Leaves an open pull request whose branch the user may want to keep.
