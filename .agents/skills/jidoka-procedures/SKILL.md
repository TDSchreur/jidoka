---
name: jidoka-procedures
description: Shared setup and continuation procedures for jidoka workflows; not a standalone workflow.
disable-model-invocation: true
---

# Jidoka procedures

Before starting or continuing a run, read [Workspace](chunks/workspace.md) and [Run reporting](chunks/run-reporting.md). After setup or continuation, run `jidoka run next <id>`, follow its instructions, then run `jidoka run advance <id>` when the stage is done. Repeat until the run is complete; follow the Rework instructions when returning to Build.
