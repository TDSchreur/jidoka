## Committing for Check

Before Check, after the appropriate iteration checks, commit everything that belongs to the change, including new files but not scratch or generated files (ignore or remove those), with a normal commit (never amend), so `git status --porcelain` is empty. `jidoka-code-review` diffs committed work only, so uncommitted changes would be missed. Pin the review: the baseline is the base commit recorded for the run (`--base-commit`; read it with `jidoka run inspect`), and the reviewed commit is `HEAD`; record the reviewed commit's short hash with `--summary check="Review pinned at <hash>"` so a new session can find it, and repeat the latest reviewed hash in the Check summary you write when the stage ends, because a new summary replaces the old one.


## Reworks

Wait for every check started in Check to report. Record its findings and resolve those you will dismiss or defer. To return to Build, use `jidoka run rework <id> --finding <finding-id> --classification <wording|behavior>`, naming one open Finding that prompted the return. Every open Finding goes into the Rework. Jidoka assigns the next number and prints the Build instructions.

Classify the whole set of fixes. Use `wording` when every fix is limited to comments, docs, docstrings, log messages or typos in internal names. Use `behavior` when any fix changes what users or callers see or rely on, including user-facing messages, public names and interfaces, outputs or logic.

Findings alone never block a run; it is blocked only while it waits for the user's decision.

Fix actionable findings in Reworks 1 and 2. In Rework 3, wording fixes can proceed without a decision. If any fixes would change behavior, use `jidoka run block <id> --reason "Decision needed on remaining Check findings"` and list those findings. Ask the user to choose one of three options:

1. Fix the behavior findings in Rework 3 too.
2. List them in the pull request and proceed with only the wording fixes.
3. Stop.

After Rework 3, or when a finding cannot be resolved, block and ask the user whether to do one more Rework, list the findings in the pull request and finish, or stop.

After any of these decisions, leave the run blocked if the user chose to stop. Otherwise do these steps in this order, because the Rework takes every Finding that is still open:

1. Resume with `jidoka run resume <id> --source "<user's decision>"`.
2. Defer the findings the user chose to leave for the pull request with `--finding-status <finding-id>=deferred:<reason>` on `run update`.
3. If the user chose another Rework, start it with `jidoka run rework`. Include `--decision "<user's authorization>"` if the user approved behavior fixes in Rework 3 or authorized any later Rework.

While fixing in Build, commit each fix and mark the Findings it resolves with `jidoka run update <id> --finding-status <finding-id>=fixed` (repeat for several). Jidoka records the commit. The next Check can reopen a Finding with `--finding-status <finding-id>=open` or record new Findings.
