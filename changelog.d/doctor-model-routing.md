## Added

- `factory doctor` checks a model-routing policy when one exists and is switched on. A new "Model routing" section reports whether the policy loads, whether Claude Code is recent enough for the Agent tool's `effort` option, whether the roles use models and efforts it accepts, and whether `CLAUDE_CODE_SUBAGENT_MODEL_FORCE`, `CLAUDE_CODE_SUBAGENT_MODEL` or `CLAUDE_CODE_EFFORT_LEVEL` would override the policy. Codex checks the command and version and skips what the factory cannot judge. A Copilot CLI policy gets one line saying model routing is not supported there.
- Two new report statuses: `warning` for a risk that does not stop the factory, and `skipped` for something that was not checked. Only a policy file that cannot be used makes the report not ready.
- `factory doctor --json` has a `model_routing` list when a policy exists, and its checks can use the new `warning` and `skipped` statuses. Reports without a policy are unchanged.
