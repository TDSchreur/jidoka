## Added

- Model routing: an optional `model-routing.json` in the factory's config folder sets the model and effort for the `strong`, `coding` and `routine` roles and says which role handles build, check and finish. `factory run start` reads it once and records the selection on the run. No workflow reads the selection yet.
- `factory run inspect` shows a run's model routing, and `--json` carries it.
- `factory run update` takes `--model-routing <task>=<role>` to override the role for a task on one run, and `--clear-model-routing` to remove the overrides.
- A policy for Copilot CLI records nothing, because Copilot gives no per-call model choice for subagents.
