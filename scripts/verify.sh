#!/bin/sh
# Run the checks CI runs, in the same order, and stop at the first failure.
# With --quick (for iterating) it skips the Windows clippy step and runs the dashboard browser tests only when the
# dashboard or its tests changed since the merge base with origin/develop. Run it without --quick before a pull request.
# Keep in sync with the `check` job in .github/workflows/ci.yml. CI lints on Linux only; the Windows clippy step below
# is an extra local check that runs when the Windows target is installed.
set -eu

missing() {
  printf '%s is not available. Install the Rust toolchain from https://rustup.rs, then run: rustup component add rustfmt clippy\n' "$1" >&2
  exit 1
}

# Prints the browser the dashboard browser tests will use; keep in sync with find_browser in tests/dashboard_browser.py.
browser() {
  if [ -n "${CHROMIUM:-}" ]; then printf '%s' "$CHROMIUM"; return 0; fi
  for name in chromium chromium-browser google-chrome; do
    command -v "$name" 2>/dev/null && return 0
  done
  return 1
}

# Prints the Windows target to lint: x86_64-pc-windows-gnu when installed, else the first installed *-pc-windows-* target.
installed_windows_target() {
  command -v rustup >/dev/null 2>&1 || return 1
  targets=$(rustup target list --installed 2>/dev/null) || return 1
  if printf '%s\n' "$targets" | grep -qx 'x86_64-pc-windows-gnu'; then
    printf '%s\n' x86_64-pc-windows-gnu
  else
    printf '%s\n' "$targets" | grep -m1 -e '-pc-windows-'
  fi
}

# True when the dashboard or its browser tests changed; also true when that cannot be determined.
dashboard_changed() {
  command -v git >/dev/null 2>&1 || return 0
  base=$(git merge-base HEAD origin/develop 2>/dev/null) || return 0
  { git diff --name-only "$base"; git ls-files --others --exclude-standard; } 2>/dev/null |
    grep -Eq '^(dashboard/|tests/.*\.py$)'
}

step() {
  printf '\n==> %s\n' "$*"
  "$@" || { printf 'FAILED: %s\n' "$*" >&2; exit 1; }
}

main() {
  quick=0
  case "${1:-}" in
    --quick) quick=1 ;;
    '') ;;
    *) printf 'usage: verify.sh [--quick]\n' >&2; exit 2 ;;
  esac
  case "$0" in
    */*) cd "${0%/*}/.." ;;
    *) cd .. ;;
  esac
  command -v cargo >/dev/null 2>&1 || missing cargo
  cargo fmt --version >/dev/null 2>&1 || missing rustfmt
  cargo clippy --version >/dev/null 2>&1 || missing clippy
  browser_tests=1
  if [ "$quick" = 1 ] && ! dashboard_changed; then browser_tests=0; fi
  if [ "$browser_tests" = 1 ]; then
    command -v python3 >/dev/null 2>&1 || { printf 'python3 is not available. Install Python 3 to run the dashboard browser tests.\n' >&2; exit 1; }
    browser >/dev/null || { printf 'Chromium is not available. Install chromium or google-chrome (or set CHROMIUM to its path) to run the dashboard browser tests.\n' >&2; exit 1; }
  fi
  step cargo fmt --check
  step cargo clippy --all-targets --locked -- -D warnings
  if [ "$quick" = 1 ]; then
    :
  elif windows_target=$(installed_windows_target); then
    step cargo clippy --all-targets --locked --target "$windows_target" -- -D warnings
  else
    printf '\n==> Skipped Windows clippy: no Windows target is installed. Add one with: rustup target add x86_64-pc-windows-gnu\n'
  fi
  step cargo test --locked
  if [ "$browser_tests" = 1 ]; then
    step python3 -m unittest discover -s tests -p 'test_*.py'
    for test in tests/dashboard-*.py; do
      step python3 "$test"
    done
  else
    printf '\n==> Skipped dashboard browser tests (quick mode, no dashboard changes)\n'
  fi
  printf '\nAll checks passed.\n'
}
main "$@"
