## Added

- `factory install`, `factory update` and `factory doctor` take a repeatable `--config-dir <path>` for the Claude agent, so one install can serve several Claude Code config folders such as `~/.claude` and `~/.claude-work`. The installers and update scripts forward `FACTORY_CLAUDE_CONFIG_DIRS`, and the dashboard and run reports read Claude session logs from every folder in it.
