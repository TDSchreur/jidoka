---
description: 'Start external command line tools so they also work on Windows'
applyTo: 'src/**/*.rs'
---

# External tools

- Windows starts a bare program name only as `<name>.exe`. Tools installed as `.cmd` or `.bat` launchers, such as the Azure CLI (`az.cmd`), must be started by the path that `find_program` returns, as `az_program()` does for `az`. Never start `az` by name.
- On Windows, `find_program` tries the `PATHEXT` extensions, and a name that already ends in one of them (case-insensitive), such as `npm.cmd`, is also tried as given. For a bare name, a file without an extension next to `az.cmd` is a Bash script that Windows cannot run, so it is never a candidate.
- Start the program with `Command`, never through a shell, so arguments are passed without re-parsing.
- When a new tool needs this, give it a resolver like `az_program()` and a `#[cfg(windows)]` test that finds and runs a fake `.cmd`; Windows CI runs it.
