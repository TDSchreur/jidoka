---
status: accepted
---

# Runs record the agent home of their sessions

This refines [ADR-0016](0016-one-home-directory-per-agent.md). The home directory for usage reporting is the one the run's agent ran in, not the one of the dashboard's environment. When the CLI records a session, it sends the agent's home folder (what `CLAUDE_CONFIG_DIR`, `CODEX_HOME` or `COPILOT_HOME` resolves to in the agent's environment) and the run stores it in `agent_homes`, keyed by agent. Token usage and `jidoka run requests` read that agent's logs there. Before this, a dashboard started with another `CLAUDE_CONFIG_DIR` than the agent found no logs and stored `not tracked`, and a finished run was never collected again.

The dashboard now reads logs from a path posted through the token-protected CLI API. That is acceptable: the CLI and the dashboard run as the same user, the path must be absolute and free of control characters, links are not followed, reading stays bounded, and only counts are kept.

When the dashboard starts, it collects once each finished run updated within the seven-day watch window that recorded sessions and whose usage is `not tracked`, so runs missed before are recovered. Finished runs are not watched after that.

## Considered Options

- **Re-collect finished runs only.** Rejected: a dashboard started in the wrong environment would read the wrong folder again.
- **Only improve the reason.** Rejected: the reason already names the scanned folders (#547), but the user still has to restart the dashboard with the right environment, and one dashboard cannot serve agents with different homes.
- **Scan every known profile.** Rejected: Jidoka cannot know every config folder a user has, and scanning several multiplies the reading for every run, which ADR-0016 moved away from.

## Consequences

- Runs recorded before this change have no `agent_homes` and are read from the dashboard's own homes, as before. No migration.
- A run update or advance from the new CLI carries `agent_homes`, which an older dashboard refuses as an unknown field, so the dashboard must be restarted with the new version.
- An agent home that is relative, such as `CLAUDE_CONFIG_DIR=claude-work`, is not recorded, because the dashboard cannot tell what it is relative to.
