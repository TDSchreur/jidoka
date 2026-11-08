#!/bin/sh
# Run the checks CI runs, in the same order, and stop at the first failure.
# Keep in sync with the `check` job in .github/workflows/ci.yml.
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

step() {
  printf '\n==> %s\n' "$*"
  "$@" || { printf 'FAILED: %s\n' "$*" >&2; exit 1; }
}

main() {
  case "$0" in
    */*) cd "${0%/*}/.." ;;
    *) cd .. ;;
  esac
  command -v cargo >/dev/null 2>&1 || missing cargo
  cargo fmt --version >/dev/null 2>&1 || missing rustfmt
  cargo clippy --version >/dev/null 2>&1 || missing clippy
  command -v python3 >/dev/null 2>&1 || { printf 'python3 is not available. Install Python 3 to run the dashboard browser tests.\n' >&2; exit 1; }
  browser >/dev/null || { printf 'Chromium is not available. Install chromium or google-chrome (or set CHROMIUM to its path) to run the dashboard browser tests.\n' >&2; exit 1; }
  step cargo fmt --check
  step cargo clippy --all-targets --locked -- -D warnings
  step cargo test --locked
  for test in tests/dashboard-*.py; do
    step python3 "$test"
  done
  printf '\nAll checks passed.\n'
}
main "$@"
