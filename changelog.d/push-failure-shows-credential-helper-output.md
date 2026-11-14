## Fixed

- `jidoka pr open` now shows all of git's output when a push is refused for missing credentials, so a crashing credential helper is visible instead of only the last `fatal:` line, and the hint points at the configured helpers as well as `gh auth login`.
