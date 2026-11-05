---
description: 'Write portable, safe POSIX shell scripts'
applyTo: '**/*.sh'
---

# Shell scripts

- Match the shebang to the dialect. Installers use `#!/bin/sh` with `set -eu`; stay POSIX there (no arrays, `[[ ]]`, or `local`). Use `set -euo pipefail` only in explicit bash scripts.
- Quote every variable expansion (`"$var"`) and use `--` before user-provided paths that could start with `-`.
- Check required tools with `command -v` and fail with a clear message on stderr and a non-zero exit.
- Create temporary files with `mktemp` and remove them with a `trap` on `EXIT HUP INT TERM`.
- Verify downloads (checksum) before executing or installing them, and replace installed files atomically (copy next to the target, then `mv`).
- Wrap the script body in a `main` function called as `main "$@"` so a truncated download does not run partially.
- Prefer `printf` over `echo` for anything with escapes or variables. Send errors to stderr.
- Do not parse `ls`; use globs or `find`. Use `$(...)`, not backticks.
- Run `shellcheck` on changed scripts when it is available and fix its findings, or explain a justified `# shellcheck disable=` inline.
