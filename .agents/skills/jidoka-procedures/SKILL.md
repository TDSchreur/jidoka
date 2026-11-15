---
name: jidoka-procedures
description: Shared setup and continuation procedures for jidoka workflows; not a standalone workflow.
disable-model-invocation: true
---

# Jidoka procedures

The workflow skills start an ordinary run themselves with `jidoka run begin`. Read [Workspace](chunks/workspace.md) and [Run reporting](chunks/run-reporting.md) when you continue a run, when `begin` fails or prints a session warning, when the user started you in a dedicated worktree, or when a `jidoka` command rejects a command or option. After continuing a run, run `jidoka run next <id>` once for the current stage's instructions. When a stage is done, run `jidoka run advance <id>` with the stage summary (see Run reporting); it prints the next stage's instructions. Repeat until the run is complete; follow the Rework instructions when returning to Build.
