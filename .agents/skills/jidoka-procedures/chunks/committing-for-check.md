## Verification

Find the project’s test and verification instructions in its agent instructions, contributing docs or build files. Follow its test scope and cadence; during iteration use targeted or quick checks appropriate to the change, unless the project or user requires more. Run all verification required before merging on the final candidate before the pull request. Reuse a passing result only while the checked code and relevant inputs are unchanged. A missing required tool or failing required check blocks the run: fix it or mark the run blocked, and never skip it. If no verification can be found, ask the user once, or say so in the pull request when running unattended.


## Committing for Check

Before Check, after the appropriate iteration checks, commit everything that belongs to the change, including new files but not scratch or generated files (ignore or remove those), with a normal commit (never amend), so `git status --porcelain` is empty. `jidoka-code-review` diffs committed work only, so uncommitted changes would be missed. Pin the review: the baseline is the base commit recorded for the run (`--base-commit`; read it with `jidoka run inspect`), and the reviewed commit is `HEAD`; record the reviewed commit's short hash with `--summary check="Review pinned at <hash>"` so a new session can find it, and repeat the latest reviewed hash in the Check summary you write when the stage ends, because a new summary replaces the old one.


## Reworks

When Check sends the run back to Build, report it as a Rework: `jidoka run update <id> build running --rework <n> --activity "Rework <n>: <findings being fixed>"`, numbering Reworks from 1 within the run and leaving the count set when Check runs again. A run gets at most three Reworks.

Fix actionable findings yourself within the Rework allowance; findings alone never block a run. Reworks 1 and 2 fix the actionable findings.

When findings remain after the second Rework, those that change only wording or documentation go into the third Rework without asking the user. Wording means text with no effect on what the program does or promises: comments, docs, docstrings, log messages, typos in internal names. A finding is behaviour when it changes what users or callers see or rely on: user-facing messages (especially when they name a different condition), public names and interfaces, outputs, or logic. If a remaining finding would change behaviour, the run needs a decision from the user, so report it as blocked: `jidoka run update <id> check blocked --blocking-reason "Decision needed on remaining Check findings"`. List those findings and ask the user whether to fix them in the third Rework too, finish with them listed in the pull request, or stop; the wording and documentation fixes still go into the third Rework unless they stop.

When findings remain after the third Rework, or a finding cannot be resolved, the run also needs a decision, so report it as blocked the same way. List the remaining findings and ask the user whether to do one more Rework, finish with the findings listed in the pull request, or stop. Follow their choice, and when work resumes clear the reason with `jidoka run update <id> <build|finish> running --clear-blocking-reason`. If they stop the run, leave it blocked.


## Changing agent skills

In Build, routed or not, when a ticket changes what an agent skill (a `SKILL.md` or the files it references) makes the agent do, decide, output or promise, use a skill-creator skill if the agent has one, and verify the change with realistic prompts run by a fresh agent that has the changed skill. Follow the project's own skill-authoring guidance where it has any. Record a `Skill check:` line in the commit message: what was run and its outcome, and when no skill-creator skill is available, say so there. Changes to wording only (typos, phrasing, formatting, links) need no check; the line says `wording only`.

