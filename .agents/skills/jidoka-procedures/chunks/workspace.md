## Run ownership

The session that starts a run coordinates it to the end: the stages in order, the `jidoka` commands, run reporting, the interview and the hand-off of reviews. Individual tasks (a ticket, a review, a lookup, the pull request text) may go to agents as the workflow and model routing say; never the whole run or several stages to one agent. This holds even when other instructions say to delegate tasks.


## Using the bundled skills

In this project's checkout, the bundled skills are in `skills/<name>/`; in an installed bundle, they are siblings of this directory. Otherwise use the corresponding installed skills. `jidoka-grill-with-docs`, `jidoka-to-spec` and `jidoka-to-tickets` can only be invoked by the user, so the agent's skill tool may refuse them: read their `SKILL.md` from that location instead of invoking them. A workflow's instructions take precedence where they differ from those skills.


## Continuing a run

When the user asks you to continue an existing run, or gives you a run ID, read its context with `jidoka run inspect <id>` (add `--json` for structured output) before changing anything, and check the recorded state against reality. Fields shown as unknown were never recorded: do not guess them.

Run `jidoka run check <id>` to compare the recorded state with the repository. It only reads. It reports each of the checks below as matching, differing with what it found, or unknown, and as not checked when it needs a worktree that is missing or differs. It exits with status 0 when nothing differs and 2 when something does. Every uncommitted change counts as a difference, so judge whether a listed change is expected. The checks are:

1. The recorded repository exists and shares its git directory with the worktree.
2. The recorded worktree exists and git lists it as a worktree.
3. That worktree is on the recorded branch.
4. The recorded base commit is an ancestor of `HEAD`.
5. The worktree has no unexpected changes, and any commits not yet pushed are listed.

Report every discrepancy and every unknown to the user, with what you found. Never reset, clean, check out over, delete or discard anything to make the state match, and do not start new work while something differs. If everything matches, read the recorded spec or saved plan from its reference (it holds the acceptance criteria), then continue in the recorded worktree from the recorded stage, at the first ticket that is not done, and record new context as you go. If something differs, ask the user how to proceed.


## Workspace

Each run must have its own fresh git worktree. Never switch branches or modify files in the user's main checkout, because the user may be working in it; the dedicated run worktree is where changes belong. Determine the project's documented PR base branch, otherwise the remote default from `git symbolic-ref refs/remotes/origin/HEAD`, and fetch it with `git fetch origin <base>`.

If the user explicitly starts this run in a fresh, dedicated worktree (for example, one provisioned by their agent environment), it may be the run's worktree. Verify that it is a linked worktree, is clean, has not been used for another run, and its `HEAD` equals `origin/<base>` after the fetch. Use a new feature branch there; if a branch is already checked out, only use it if it was created for this run and is not the base branch. Do not reset, clean, or discard changes to make a worktree eligible. In that case `jidoka run begin` is not used, because it makes its own worktree: run `jidoka run start`, then record the worktree, branch and base commit with `jidoka run update`, as described under Run reporting. Also make sure `.jidoka/` and `.scratch/` are listed in that worktree's `.gitignore`, adding the missing ones and committing the change with the first commit. If any condition is not met, create a new worktree instead, or stop and explain the constraint if the environment cannot do so.

Otherwise create the run and its workspace with `jidoka run begin "<short task title>" --agent <agent> --mode <the workflow's name> --base <base>`, where `<base>` is the project's documented PR base branch (without `--base` it uses the remote default). It creates the run, fetches the base, creates the worktree from `origin/<base>` on a new branch named from the title (`--branch <name>` chooses one), records mode, worktree, branch and base commit, and prints the run ID, worktree path, branch and base commit; `--json` prints them as data. Keep the printed run ID. If it ends with a blocked run and a retry command, fix what the reason says, or ask the user, and then run the retry command: it finishes what the failed attempt left and never deletes, resets or cleans anything. `begin` also adds `.jidoka/` and `.scratch/` to the worktree's `.gitignore` when they are missing and reports the added entries (`gitignore_added` in `--json`) or a warning; when it reports added entries, commit `.gitignore` with the first commit. If the worktree cannot be created, mark the run blocked with the reason and stop.

Run every later stage from the run's worktree: Define files, Build, checks, `jidoka run update` commands, commit, push, and the pull request. Respect the environment's permissions: a writable worktree does not necessarily grant write access to shared Git metadata or network access. Report any required permission rather than assuming it is available.

