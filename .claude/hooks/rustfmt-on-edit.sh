#!/bin/sh
# PostToolUse hook: format an edited Rust file so `cargo fmt --check` does not fail later.
# Never blocks the edit: a file that does not parse is left for the compiler to report.
set -u

main() {
  input=$(cat)
  path=$(printf '%s' "$input" | sed -n 's/.*"file_path"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1)
  case "$path" in
    *.rs) [ -f "$path" ] && command -v rustfmt >/dev/null 2>&1 && rustfmt --edition 2024 -- "$path" >/dev/null 2>&1 ;;
  esac
  exit 0
}

main "$@"
