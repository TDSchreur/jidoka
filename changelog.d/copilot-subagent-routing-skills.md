## Changed

- The bundled skills tell an agent on Copilot CLI how to start a routed review agent: pass the recorded model and effort as the `task` tool's `model` and `reasoning_effort`, and check the model that actually ran in the session's `events.jsonl`.
