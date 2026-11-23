---
name: post-merge-cleanup
description: Finish a merged jidoka run: remove its worktree and local branch, then update the local jidoka install. Use after a run's pull request has merged. Takes the run ID.
disable-model-invocation: true
argument-hint: <run-id>
---

# Post-merge cleanup

Run both steps from the main checkout (the first entry of `git worktree list --porcelain`), not from the run's worktree. The run ID is the argument the user gave; if there is none, ask for it.

Invoking this skill is the user's authorization to remove the run's worktree and local branch.

1. Remove the run's worktree and local branch:

   ```sh
   jidoka run cleanup <run-id>
   ```

   Jidoka checks that the recorded pull request has merged before it removes anything. If it refuses, report why and stop; do not remove the worktree or branch by hand.

2. Pull `develop`, rebuild, reinstall the `jidoka` command and the skills for all agents, and restart the dashboard:

   ```sh
   JIDOKA_AGENT=all scripts/update-local.sh
   ```

Report the output of both steps. If a step fails, say which one and show its error.
