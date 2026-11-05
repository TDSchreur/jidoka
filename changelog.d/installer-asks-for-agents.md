## Changed

- `install.sh` and `install.ps1` now ask which agents to install the skills for (Codex, Claude Code, GitHub Copilot, one or more) instead of defaulting to Codex. `FACTORY_AGENT` accepts a comma-separated list to skip the prompt; without it and without a terminal, the installer stops before installing anything.
- `factory install --agent` accepts a comma-separated list, such as `codex,claude`, and checks every name before installing.
