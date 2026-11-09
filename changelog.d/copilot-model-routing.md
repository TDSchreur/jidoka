## Added

- Model routing supports Copilot CLI: add an `agents.copilot` entry to the policy and a run started with `--agent copilot` records it, shows it in `factory run inspect` and the dashboard help, and routes each Stage through the `task` tool's `model` and `reasoning_effort`. Copilot entries must use exact model IDs (not Claude Code aliases such as `sonnet`) and an effort from `none` to `max`. `factory doctor --agent copilot --create-model-routing` writes a Copilot starter.

## Changed

- The "Model routing is not supported on Copilot CLI" notice is gone, and an `agents.copilot` entry is no longer an error. A run in Copilot CLI without an entry is not routed, like any other agent.
