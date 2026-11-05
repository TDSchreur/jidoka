## Added

- Token usage per run, stage, model and ticket, read from the Claude Code, Codex and Copilot session logs. A session counts for a run only when its id was recorded on the run (Claude Code's is recorded automatically; `--session <id>` for others) or its log names the run id; log entries are assigned to the stage, rework and ticket the run was in at that moment, with rework and work with no ticket in their own buckets, and subagent usage included once. Usage that two runs could both claim is not divided by guessing.
- Counts are normalized to input (excluding cache reads), cache read, cache write and output, with reasoning inside output. The headline "fresh tokens" are input plus cache writes plus output. Unreported counts are `n/a`, never zero, and usage is `tracked`, `partial` or `not tracked` with the reason.
- `factory run stats <id>` and `factory run stats --all` print the breakdown, per-model totals and the coverage limits. The dashboard shows fresh tokens on cards and the stage/model/ticket breakdown in the run detail.
- The dashboard collects usage in the background when a run changes stage, rework, ticket or session and when it finishes, without delaying or failing an update. Opt out with `factory dashboard --no-usage` or `FACTORY_USAGE=off`. Only counts and identifiers are stored, never conversation text.
- Runs record a small timeline of stage, rework and ticket changes, their finish time and their session ids. Runs from before this show `not tracked`.

Claude Code and Codex logs were inspected on a real machine; all three formats are fixture-tested. Copilot's usage events could not be checked against real logs, and whether Codex's `cache_write_input_tokens` is part of its input is unvalidated.
