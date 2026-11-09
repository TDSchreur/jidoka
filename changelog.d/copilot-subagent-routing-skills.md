## Changed

- The bundled skills tell an agent on Copilot CLI how to start a routed agent and how to check which model it ran on, including where `$COPILOT_HOME` puts the session's `events.jsonl`. The instructions live in `factory-procedures`, and `factory-code-review` points there for every host's routing rules and the fallback report. When Copilot fell back to the session's model, the report says "model routing not applied: Copilot ran <actual> instead of <requested>".
