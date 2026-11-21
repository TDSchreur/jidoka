---
status: accepted
---

# `advance` and `begin` print the next Stage's instructions

The coordinator agent re-reads its whole context on every model request: a median of about 92k tokens over 17 Claude Code runs, most of it cache reads. Each request saved is about 92k cache-read tokens, while the instruction text itself is small. The logs showed requests spent only on bookkeeping: a `jidoka run next` after each `jidoka run advance`, a `jidoka run update` for the stage summary or the new stage's activity, a `jidoka run next` after `jidoka run begin`, separate reads of the shared procedures before Define, and a separate read of the pull request and CI procedure in Finish.

So `jidoka run advance` prints what `jidoka run next` prints for the new Stage (`--json`: the same object), and takes `--summary`, `--activity` and `--clear-current-ticket`, which it originally recorded before checking the gates. The current contract saves advance metadata with the stage transition. See [ADR-0019](0019-record-run-lifecycle-completion-and-stage-exit-evidence.md#lifecycle-requests). `jidoka run begin` prints the Define Stage's instructions after the workspace (`--json`: `next`), except with a session warning. A done run has no Stage instructions. Finish's instructions include the pull request and CI procedure, which moved from `jidoka-procedures/references/` to `chunks/`. The workflow skills and the selector carry the start command, so the shared procedures are read only to continue a run or when `begin` fails or warns.

## Considered Options

- **A new combined command, such as `jidoka run step`.** Rejected: a second way to do the same thing, and the existing commands would still cost the extra requests.
- **Print the instructions only with a flag, such as `advance --next`.** Rejected: every agent would pass it, so it is the default without an off switch anyone needs; `jidoka run advance` output is read by agents, not scripts.
- **Record the summary after advancing.** Rejected: after the last advance the run is done, and an update that sends status `running` would reopen it.
- **Inline `jidoka-pr` into Finish as well.** Rejected: Finish usually hands the pull request text to another agent, which reads the skill itself, so inlining would add text to the coordinator's context without saving a read.

## Consequences

- `jidoka run advance` output changes from one line to the full Stage instructions, and its `--json` output is new. Scripts that matched the single line need the first line only.
- As of ADR-0019, the summary and activity given to `advance` are saved only when the transition succeeds.
- The Finish instructions are about 8 KB longer; agents read that text in Finish anyway, usually once or twice.
- `jidoka-procedures/references/pull-request-and-ci.md` no longer exists; it is `jidoka-procedures/chunks/pull-request-and-ci.md`.
