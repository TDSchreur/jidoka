#!/bin/sh
# Stop hook: when Rust or dashboard files changed, run `scripts/verify.sh --quick` and show failures to the user.
# Advisory: it never blocks Claude from stopping (exit 1 is a non-blocking error), and it skips the run Claude Code
# triggers when it continues because of a Stop hook. Set JIDOKA_SKIP_STOP_VERIFY=1 to turn it off.
set -u

main() {
  [ "${JIDOKA_SKIP_STOP_VERIFY:-}" = 1 ] && exit 0
  input=$(cat)
  case "$input" in *'"stop_hook_active":true'* | *'"stop_hook_active": true'*) exit 0 ;; esac

  cd -- "${CLAUDE_PROJECT_DIR:-.}" || exit 0
  [ -x scripts/verify.sh ] || exit 0
  changed=$(git status --porcelain 2>/dev/null | cut -c4-) || exit 0
  printf '%s\n' "$changed" | grep -Eq '(\.rs|^Cargo\.(toml|lock)|^build\.rs|^dashboard/|^tests/dashboard.*\.py)$|^(src|dashboard)/' || exit 0

  out=$(scripts/verify.sh --quick 2>&1) && exit 0
  printf 'scripts/verify.sh --quick failed:\n%s\n' "$(printf '%s\n' "$out" | tail -n 40)" >&2
  exit 1
}

main "$@"
