---
name: jidoka-full
description: Detailed jidoka workflow with project-directed testing and risk-based reviews. Usually entered from `jidoka`; use directly only when the user names jidoka-full.
---

# Jidoka Full

To start a new run, find the project's documented pull request base branch (otherwise the remote default from `git symbolic-ref refs/remotes/origin/HEAD`) and run `jidoka run begin "<short task title>" --agent <agent> --mode jidoka-full --base <base>`, where `<agent>` is the coding agent you are running in: `claude`, `codex` or `copilot`. If the dashboard is not running, start `jidoka dashboard` as a background process and retry once. `begin` creates the run and its own git worktree, and prints the run ID, the worktree and the Define stage's instructions. Do every later step in that worktree, never in the user's checkout, and commit `.gitignore` with the first commit when `begin` says it added entries to it.

Follow the printed instructions. When a stage is done, run `jidoka run advance <id> --summary <stage>="<one or two sentences>" --activity "<what the next stage does>"`: it records both, checks the gates and prints the next stage's instructions, so no separate `jidoka run next` is needed. Repeat until the run is done.

Read [jidoka-procedures](../jidoka-procedures/SKILL.md) instead when you continue an existing run, when `begin` fails or prints a session warning, when the user started you in a dedicated worktree, or when a `jidoka` command rejects a command or option.
