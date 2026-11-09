## Added

- `factory run begin` warns when the current agent session is already recorded on another run, and the factory skill tells you to start each run in a fresh session or after `/clear`. The warning names the run id and title and is in `--json` as `session_warning`; it never fails the command.
