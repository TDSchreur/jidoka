## Fixed

- A `jidoka run update` that an agent runs from inside a script tool (such as Claude Code's REPL) now counts as the session updating the run, so `jidoka run requests` no longer says no session updated a run that shows token usage.
- `jidoka run requests` now notes a recorded Codex session log that is too large to read, as the token usage does, and the token usage notes a recorded Codex log that cannot be read even when no other session was found for the run.
