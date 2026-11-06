## Added

- `factory run requests <id>` (with `--json`) shows a run's model requests per category, such as commit, verification, CI watch and reading, with the input, cache read, cache write and output tokens of each. It reads the Claude Code session logs on demand and keeps only categories and counts.
- `factory run requests` counts reporting that is chained to other commands separately from the categories, shows how many requests fit more than one category, and gives the tool output size per category.
- `factory run requests` includes subagent requests on their own lines and says which sessions are coordinator-only. Codex and Copilot sessions are counted with the category "not categorized".
- `factory run requests --all` pools the completed runs and shows, per category, the median, quartiles, minimum and maximum of requests per run, with the pooled share of requests and fresh tokens. `--include-partial` adds partial and open runs, `--exclude <id>` drops a run, and the runs it leaves out are listed with the reason.
