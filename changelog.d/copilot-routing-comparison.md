## Changed

- `factory run inspect` compares a Copilot run's routing selection with the models and efforts its logs show, like for Claude Code and Codex: a model or effort that never appears is a `MISMATCH`, partial usage says "not observed", and a call without a recorded effort says "effort not recorded".
