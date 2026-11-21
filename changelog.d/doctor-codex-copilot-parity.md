## Changed

- `jidoka doctor` checks Codex models and efforts against Codex's own model catalog (`models_cache.json`), and warns when a Copilot CLI settings file pins a subagent's model or effort. Codex `overrides` and Copilot model availability stay unchecked ([ADR-0020](docs/adr/0020-doctor-checks-what-agents-expose-and-skips-the-rest.md)).
- `docs/usage-tracking.md` now says session ids of Codex and Copilot CLI are recorded automatically too.
