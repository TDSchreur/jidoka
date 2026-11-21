---
status: accepted
---

# Doctor checks what each agent exposes and skips the rest

`jidoka doctor` checks model routing for Claude Code in depth, and now for Codex and Copilot CLI as far as they expose data. Codex models and efforts are checked against the catalog in `models_cache.json`, read best-effort. Copilot CLI `overrides` reads the settings files that can pin a subagent's model and effort. What cannot be checked reliably stays `skipped` with the reason, instead of passing or guessing.

## Considered Options

- **Keep a list of Codex models in Jidoka.** Rejected: it goes stale with every Codex release.
- **Treat `models_cache.json` as a stable interface.** Rejected: it is an internal cache that Codex writes after it first fetches the list, so a missing file or an unexpected shape is `skipped`, never a failure.
- **Check Codex `overrides`.** Not possible: no setting is known that replaces the model or effort passed when a subagent is spawned.
- **Check whether a Copilot account can use a model.** Not possible: Jidoka has no way to ask. `jidoka run inspect` shows the models that were observed.
- **Claim a Copilot subagent pin beats the routed model.** Rejected: unverified, so the warning says it can.

## Consequences

- Codex `overrides` and Copilot model availability stay unchecked; the docs say so.
- A Codex catalog that changes shape turns `model` and `effort` into `skipped`, not into a false warning.
