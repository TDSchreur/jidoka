---
status: accepted
---

# `run advance --skip-ci` skips only the CI gate

When CI cannot be read, for example because the host answers with an error or a status Jidoka does not understand, a run could only leave Finish with `--force --reason`, which skips every gate, including the Finish summary check of the other gates and the pull request checks. `jidoka run advance <id> --skip-ci --reason "<text>"` skips only the live CI check when leaving Finish. The Finish summary still applies, and the exit is refused outside Finish.

The run record gains a completion `kind` of `ci-skipped` with the `head` whose CI was not checked and the `reason`, next to `ci`, `merged` and `override` ([ADR-0019](0019-record-run-lifecycle-completion-and-stage-exit-evidence.md)). The stage exit is `accepted` with the CI gate `bypassed`, and the reason is also saved as a run report. The advance request gains a boolean `skip_ci`. Records and clients from before this change never contain it, so nothing needs migrating.

Rejected alternatives:

- A generic `--skip-gate <name>`: only the CI gate depends on an outside service, and the other gates can always be met, so a general option would invite skipping gates that should be fixed.
- Recording the skip as an `override`: it hides that the other gates passed, which is the point of the narrower option.
- Skipping the pull request state and head checks too: the pull request can still be read in the cases that motivated this, and those checks catch a merged or changed pull request.
