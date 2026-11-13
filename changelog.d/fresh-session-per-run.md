## Changed

- The `jidoka run begin` session warning now names the command for a fresh session in the agent it runs in: `/clear` in Claude Code, `/new` in Codex and `/clear` in Copilot CLI. The workflow skills stop and ask the user when the warning appears, and the Finish report tells the user to start the next run in a fresh session.
