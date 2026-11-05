#!/bin/sh
# Run the checks CI runs, in the same order, and stop at the first failure.
# Keep in sync with the `check` job in .github/workflows/ci.yml.
set -eu

missing() {
  printf '%s is not available. Install the Rust toolchain from https://rustup.rs, then run: rustup component add rustfmt clippy\n' "$1" >&2
  exit 1
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
  step cargo fmt --check
  step cargo clippy --all-targets --locked -- -D warnings
  step cargo test --locked
  printf '\nAll checks passed.\n'
}
main "$@"
