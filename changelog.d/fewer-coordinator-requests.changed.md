## Changed

- `jidoka run advance` prints the next stage's instructions, as `jidoka run next` does (`--json` prints the same data), and takes `--summary <stage>=<text>`, `--activity <text>` and `--clear-current-ticket`, so leaving a stage takes one command instead of three. A done run has no stage instructions. Scripts that read the old one-line output of `advance` should use only its first line ([ADR-0010](docs/adr/0010-advance-and-begin-print-the-next-stage.md)).
- `jidoka run begin` prints the Define stage's instructions after the workspace (`next` in `--json`), unless it prints a session warning.
- The workflow skills start a run themselves, so the shared procedures are read only to continue a run or when `begin` fails or warns. Finish's instructions include the pull request and CI procedure, which moved from `jidoka-procedures/references/pull-request-and-ci.md` to `jidoka-procedures/chunks/pull-request-and-ci.md`.
