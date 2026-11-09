## Added

- `factory doctor` checks a Copilot CLI policy like the other agents: its `model` line passes for a loading entry (and says that account availability is not checked), and a Stage without an `effort` is a warning, not a failure.

## Changed

- The starter model-routing policy written by `factory install`, `factory update` and `factory doctor --create-model-routing` now has an entry for Copilot CLI as well as Claude Code and Codex.
