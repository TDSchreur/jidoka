---
status: accepted
---

# Finish is routed in every mode, and starters offer a second reviewer and complex models

Fast runs and small changes used to keep Finish on the coordinator, which is often the most expensive model, even though the pull request text and CI repair are cheap work for the Finish model. Finish is now routed in every mode; Define still stays on the coordinator in Fast and small runs, because the interview is a conversation. The starter policy also changes: the Copilot Check stage uses `gpt-6.2-astra` (a second opinion from another model family, as Codex already does), and every agent entry gets a `complex` map offering a stronger model at high effort for Define and Build.

## Considered Options

- **Keep Finish on the coordinator for Fast and small runs.** Rejected: it gave up the Finish saving on the runs that are most common.
- **Route Fast and small Define too.** Rejected: the work is mostly the user conversation, and a hand-off costs more than it saves.
- **Add `complex` for Copilot only.** Rejected: Claude, Codex and Copilot get the same level of support.

## Consequences

- Existing `model-routing.json` files are replaced by `jidoka install`; see [ADR-0017](0017-install-refreshes-model-routing-defaults.md).
- Check on Copilot now needs `gpt-6.2-astra` to be available to the account; an unavailable model falls back to the session's model and `jidoka run inspect` shows it.
