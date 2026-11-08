## Added

- The dashboard detects usage limits and context compactions in Claude Code logs. It blocks a running run with "Usage limit reached" or "Context compaction in progress" and sets it running again when work resumes, recording Blocked and Resumed events. It never changes a block the agent set, and keeps its own block in line with the latest pause. It checks working runs about once a minute, without an agent update. Codex and Copilot logs are not checked.
- Run details show pauses and idle gaps (more than 10 minutes without activity) in the pass that holds them, and `factory run inspect` lists them under "Pauses".

## Changed

- A usage-limit message in a Claude Code log is no longer counted as a model call or reported as a skipped usage line.
