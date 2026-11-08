---
name: factory
description: Assess a software change and select the appropriate factory execution workflow. Use when the user asks to build a feature, fix a bug, or continue a run (run ID); picks Fast, Balanced or Full.
---

# Factory workflow selection

Start each run in a fresh agent session or after `/clear`, and heed the warning `factory run begin` prints when the session already belongs to another run.

Assess the request before starting a workflow. Identify the intended outcome and constraints, read applicable project instructions, and inspect enough relevant code or documentation to understand the change.

Apply these rules in order:

1. If the request is analysis-only or prohibits implementation, respect that scope and provide an assessment without starting a run.
2. If it continues a run, inspect it with `factory run inspect` and follow [factory-procedures](../factory-procedures/SKILL.md). Preserve its recorded mode and context. Legacy `factory` means Full; if the mode was not recorded, report it as unknown, resolve the missing context through the continuation procedure, and stop until it is resolved. Do not guess or create a replacement run.
3. Honor an explicit choice of execution workflow, subject to project instructions and permissions.
4. Choose Fast for renames, deletions (of code, files, commands, or documentation), and documentation-only changes when they are mechanical and easy to verify, or Balanced when references or behavior need tests and review. Never choose Full for them unless the user names it.
5. Choose Full for substantial uncertainty, interacting changes, sensitive consequences, or a concrete need for detailed planning and TDD. Authentication, authorization, secrets, destructive operations (those that destroy user data or production state, not removing code or files), and migrations deserve particular attention.
6. Choose Fast for work that is clearly bounded, understood, easy to verify, and low risk.
7. Choose Balanced for ordinary features and fixes, or when the evidence does not clearly justify either extreme.

Consider consequences and evidence rather than file count. Materially unresolved requirements or architecture favor Full; limited evidence alone defaults to Balanced. Ask a focused question only when its answer changes the outcome or prevents a responsible choice. Do not ask for approval of a routine selection.

Tell the user the selected workflow and concrete reason in one or two sentences. Then read and follow its skill in the same session: [factory-full](../factory-full/SKILL.md), [factory-balanced](../factory-balanced/SKILL.md), or [factory-fast](../factory-fast/SKILL.md). Carry forward the request, constraints, existing plan or spec, and assessment. The selected workflow owns run creation and all subsequent stages. Do not create a selector run, invoke the selector recursively, or switch modes automatically after a run starts. If later discoveries invalidate the plan, follow the active workflow's clarification and blocking procedures without silently removing promised tests or reviews.

Examples: a spelling fix or a confirmed mechanical link cleanup suits Fast. Renaming a module across the codebase or deleting an unused command suits Fast or Balanced, and a docs-only rewrite suits Fast. A small feature with clear acceptance criteria or a familiar local bug whose behavior needs tests and review suits Balanced. A small change to configuration read at startup suits Balanced when its effect on startup needs tests and review. A redesign of authentication, a one-line authorization decision, or unresolved domain rules that materially affect architecture suits Full. An explicit Balanced or Fast request takes precedence over these examples.
