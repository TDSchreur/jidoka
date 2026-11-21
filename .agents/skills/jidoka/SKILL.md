---
name: jidoka
description: Assess a software change and select the appropriate jidoka execution workflow. Use when the user asks to build a feature, fix a bug, or continue a run (run ID); picks Fast, Balanced or Full.
---

# Jidoka workflow selection

In Codex, when requesting escalation for any Jidoka command eligible for reusable approval, set the tool’s `prefix_rule` to the specific command verb, for example `["jidoka", "run", "check"]`. Keep run IDs and other changing arguments in the command itself.

Start each run in a fresh agent session (`/clear` in Claude Code, `/new` in Codex, `/clear` in Copilot CLI). A long session re-reads its whole context on every turn, which makes later runs expensive. When `jidoka run begin` prints a session warning, stop and follow the Workspace procedure instead of carrying on.

Assess the request before starting a workflow. Identify the intended outcome and constraints, read applicable project instructions, and inspect enough relevant code or documentation to understand the change.

Apply these rules in order:

1. If the request is analysis-only or prohibits implementation, respect that scope and provide an assessment without starting a run.
2. If it continues a run, inspect it with `jidoka run inspect` and follow [jidoka-procedures](../jidoka-procedures/SKILL.md). Preserve its recorded mode and context. If the mode was not recorded, report it as unknown, resolve the missing context through the continuation procedure, and stop until it is resolved. Do not guess or create a replacement run.
3. Honor an explicit choice of execution workflow, subject to project instructions and permissions.
4. Choose Fast for renames, deletions (of code, files, commands, or documentation), and documentation-only changes when they are mechanical and easy to verify, or Balanced when references or behavior need tests and review. Never choose Full for them unless the user names it.
5. Choose Full for substantial uncertainty, interacting changes, sensitive consequences, or a concrete need for detailed planning and rigorous testing. Authentication, authorization, secrets, destructive operations (those that destroy user data or production state, not removing code or files), and migrations deserve particular attention.
6. Choose Fast for work that is clearly bounded, understood, easy to verify, and low risk.
7. Choose Balanced for ordinary features and fixes, or when the evidence does not clearly justify either extreme.

Consider consequences and evidence rather than file count. Materially unresolved requirements or architecture favor Full; limited evidence alone defaults to Balanced. Ask a focused question only when its answer changes the outcome or prevents a responsible choice. Do not ask for approval of a routine selection.

Tell the user the selected workflow and concrete reason in one or two sentences. Then start it in the same session. Its skill, [jidoka-full](../jidoka-full/SKILL.md), [jidoka-balanced](../jidoka-balanced/SKILL.md) or [jidoka-fast](../jidoka-fast/SKILL.md), says only what follows, with its own name as the mode, so you need not read it:

> To start a new run, find the project's documented pull request base branch (otherwise the remote default from `git symbolic-ref refs/remotes/origin/HEAD`) and run `jidoka run begin "<short task title>" --agent <agent> --mode <jidoka-full|jidoka-balanced|jidoka-fast> --base <base>`, where `<agent>` is the coding agent you are running in: `claude`, `codex` or `copilot`. If the dashboard is not running, start `jidoka dashboard` as a background process and retry once. `begin` creates the run and its own git worktree, and prints the run ID, the worktree and the Define stage's instructions. Do every later step in that worktree, never in the user's checkout, and commit `.gitignore` with the first commit when `begin` says it added entries to it.
>
> Follow the printed instructions. When a stage is done, run `jidoka run advance <id> --summary <stage>="<one or two sentences>" --activity "<what the next stage does>"`: it records both, checks the gates and prints the next stage's instructions, so no separate `jidoka run next` is needed. Repeat until the run is done.
>
> Read [jidoka-procedures](../jidoka-procedures/SKILL.md) instead when you continue an existing run, when `begin` fails or prints a session warning, when the user started you in a dedicated worktree, or when a `jidoka` command rejects a command or option.

Carry forward the request, constraints, existing plan or spec, and assessment. The selected workflow owns run creation and all subsequent stages, and this session runs them all (Run ownership in [jidoka-procedures](../jidoka-procedures/SKILL.md)): even when other instructions say to delegate tasks, delegate only individual tasks. Do not create a selector run, invoke the selector recursively, or switch modes automatically after a run starts. If later discoveries invalidate the plan, follow the active workflow's clarification and blocking procedures without silently removing promised tests or reviews.

Examples: a spelling fix or a confirmed mechanical link cleanup suits Fast. Renaming a module across the codebase or deleting an unused command suits Fast or Balanced, and a docs-only rewrite suits Fast. A small, understood, low-risk feature with clear acceptance criteria can suit Fast, including when it needs behavior tests. A familiar local bug whose behavior needs review suits Balanced. A small change to configuration read at startup suits Balanced when its effect on startup needs tests and review. A redesign of authentication, a one-line authorization decision, or unresolved domain rules that materially affect architecture suits Full. An explicit Balanced or Fast request takes precedence over these examples.
