## Added

- When a run has a model-routing selection for `check`, the `factory-full` and `factory-balanced` workflows start the review agents of `code-review` with that role's model and effort: on Claude Code through the Agent tool's `model` and `effort` on fresh subagents, on Codex through its subagent settings. The coordinating session keeps its own model, and the stage summary says what was applied or why routing was not. Without a selection, runs behave as before. `factory-fast` has no Check and is unchanged.
