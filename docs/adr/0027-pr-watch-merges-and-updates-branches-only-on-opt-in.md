---
status: accepted
---

# `jidoka-pr-watch` merges and updates branches only on opt-in

`jidoka-pr-watch` used to never merge a pull request and left a branch that is only behind its base alone. Two chores are now available, both off unless the user asks for them when starting the watch: squash-merging a ready pull request, and merging the base into a behind branch. Merging is one-way, so the default stays the cautious one and the user's own words switch it on.

A pull request is merged only when it is open, not draft, free of conflicts, with every check and blocking policy passed, no unresolved threads (declined ones included) and nothing pushed to it in the same pass. The merge is squashed, pinned to the checked head commit, never bypasses a policy and never deletes the branch. A behind branch gets `git merge origin/<base>` in the worktree, the project's verification and a normal push; nothing is force-pushed.

Rejected alternatives:

- Merge by default: a watcher running unattended on the user's credentials would then make an irreversible decision they never asked for.
- GitHub's `update-branch` API: Azure DevOps has no equivalent, and the parity rule asks for the same behaviour on both hosts. A local merge works on both.
- Rebasing a behind branch: it forces a push and rewrites reviewers' diffs for no benefit when the pull request is squash-merged anyway.
- Host auto-merge or auto-complete: it depends on branch rules every repository does not have, and the skill could not check the thread and head conditions itself.
- Other merge strategies: squash gives one commit per pull request and is the choice the user named; a setting can be added when someone needs another.
