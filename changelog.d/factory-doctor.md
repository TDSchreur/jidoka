## Added

- `factory doctor` reports, read-only, whether the repository (Git, `origin`, default branch, setup docs, documented verification commands) and each agent (Codex, Claude Code, Copilot: command on `PATH`, bundled skills installed) are ready, with a fix for every problem. It exits successfully when the project checks pass and at least one agent is usable. `--agent` checks and requires one agent, and `--json` prints structured output.
