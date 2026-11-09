## Fixed

- Copilot token usage now has an estimated cost: model ids written with a dot, such as `claude-sonnet-5.5`, match the built-in prices.
- A session counts for a run when its log names the run in `factory pr open`, `pr sync`, `pr wait` or `run check`, not only `run update`, `run inspect` or `run stats`. The command must start the text or follow whitespace, so `xpr open <id>` does not count.
