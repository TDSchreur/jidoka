## Changed

- `factory run requests` reads the commands in Codex rollout files and gives Codex requests the same categories and tool output sizes as Claude Code requests. A Codex session claims a run through a recorded session id or an executed `factory run update`; a mention of the run id elsewhere in its log no longer claims it. Only Copilot requests are still shown as not categorized.
