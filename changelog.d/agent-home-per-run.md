## Fixed

- Token usage and `jidoka run requests` now read a run's session logs from the agent home the run's agent ran in. `jidoka run begin`, `run start`, `run update` and `run advance` record it with the session id, as `agent_homes` on the run, from `CLAUDE_CONFIG_DIR`, `CODEX_HOME` or `COPILOT_HOME` in the agent's environment. A dashboard started with another `CLAUDE_CONFIG_DIR` no longer stores `not tracked` for such runs. `jidoka run update` accepts `--agent-home <agent>=<absolute-path>` to record one yourself. Runs recorded before this have no agent homes and are read as before.
- When the dashboard starts, it collects once more each finished run of the last seven days that recorded sessions but has usage `not tracked`, so runs it missed before are recovered.
- Restart the dashboard with the updated CLI: a dashboard older than this refuses run updates and advances from the new CLI, because they carry `agent_homes`.
