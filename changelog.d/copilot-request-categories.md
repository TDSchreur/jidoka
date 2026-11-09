## Changed

- `factory run requests` now categorizes Copilot requests from their tool calls (shell commands, file reads and edits, subagent runs) with the same rules as Claude Code and Codex, counts the size of their tool output, and shows subagent requests on their own lines. A count the log does not report stays `n/a`.
