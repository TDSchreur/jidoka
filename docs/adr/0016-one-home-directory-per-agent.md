# Jidoka uses one home directory per agent

Claude Code previously had Jidoka-specific multi-directory settings for skill installation, diagnostics, and session-log discovery. Use each agent's native home override for a single directory so Claude Code, Codex, and Copilot have equivalent configuration. A shared multi-directory feature would give all agents parity but would expand the configuration Jidoka needs to manage.

The active environment selects the Claude profile for install, doctor, skill refresh, and usage reporting. The install record stores the selected agents and binary version.
