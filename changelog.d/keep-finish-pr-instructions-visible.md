## Changed

- Stage instructions show the Codex command approvals only to Codex runs, or when the run has no recorded agent, so the Finish steps stay near the top for Claude and Copilot.
- Finish now opens with the reminder to use `jidoka pr open` and `jidoka pr wait` instead of calling `gh` or `az`, and the reporting procedures tell agents to read the whole output of `jidoka run advance` and `jidoka run next` without cutting it.
