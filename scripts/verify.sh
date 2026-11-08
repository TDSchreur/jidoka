#!/bin/sh
# Run the checks CI runs, in the same order, and stop at the first failure.
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

WINDOWS_TARGET=x86_64-pc-windows-gnu

windows_target_installed() {
  command -v rustup >/dev/null 2>&1 || return 1
  targets=$(rustup target list --installed 2>/dev/null) || return 1
  case "
$targets
" in
    *"
$WINDOWS_TARGET
"*) return 0 ;;
    *) return 1 ;;
  esac
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
  if windows_target_installed; then
    step cargo clippy --all-targets --locked --target "$WINDOWS_TARGET" -- -D warnings
  else
    printf '\n==> Skipped Windows clippy: target %s is not installed. Add it with: rustup target add %s\n' "$WINDOWS_TARGET" "$WINDOWS_TARGET"
  fi
  step cargo test --locked
  for test in tests/dashboard-*.py; do
    step python3 "$test"
  done
  printf '\nAll checks passed.\n'
}
main "$@"
