## Committing for Check

Before Check, after the appropriate iteration checks, commit everything that belongs to the change, including new files but not scratch or generated files (ignore or remove those), with a normal commit (never amend), so `git status --porcelain` is empty. `jidoka-code-review` diffs committed work only, so uncommitted changes would be missed. Pin the review: the baseline is the base commit recorded for the run (`--base-commit`; read it with `jidoka run inspect`), and the reviewed commit is `HEAD`; record the reviewed commit's short hash with `--summary check="Review pinned at <hash>"` so a new session can find it, and repeat the latest reviewed hash in the Check summary you write when the stage ends, because a new summary replaces the old one.


## Reworks

A Rework is the one stage change that does not use `jidoka run advance`: while the run is in Check, run `jidoka run update <id> running --rework <n> --activity "Rework <n>: <findings being fixed>"`, and the run returns to Build. Number Reworks from 1 within the run and leave the count set when Check runs again. A run gets at most three Reworks.

Fix actionable findings yourself within the Rework allowance; findings alone never block a run. Reworks 1 and 2 fix the actionable findings.

When findings remain after the second Rework, those that change only wording or documentation go into the third Rework without asking the user. Wording means text with no effect on what the program does or promises: comments, docs, docstrings, log messages, typos in internal names. A finding is behaviour when it changes what users or callers see or rely on: user-facing messages (especially when they name a different condition), public names and interfaces, outputs, or logic. If a remaining finding would change behaviour, the run needs a decision from the user, so report it as blocked: `jidoka run update <id> blocked --blocking-reason "Decision needed on remaining Check findings"`. List those findings and ask the user whether to fix them in the third Rework too, finish with them listed in the pull request, or stop; the wording and documentation fixes still go into the third Rework unless they stop.

When findings remain after the third Rework, or a finding cannot be resolved, the run also needs a decision, so report it as blocked the same way. List the remaining findings and ask the user whether to do one more Rework, finish with the findings listed in the pull request, or stop. Follow their choice, and when work resumes clear the reason with `jidoka run update <id> running --clear-blocking-reason`. If they stop the run, leave it blocked.


