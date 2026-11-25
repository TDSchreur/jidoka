#!/bin/sh
# PreToolUse hook: deny hand edits to generated or vendored paths: target/, third-party/ and Cargo.lock at the repository root.
# Cargo owns Cargo.lock and target/; third-party/ is vendored. Set JIDOKA_ALLOW_PROTECTED_EDITS=1 when an edit is intended.
set -eu

main() {
  [ "${JIDOKA_ALLOW_PROTECTED_EDITS:-}" = 1 ] && exit 0
  input=$(cat)
  # The tool input is JSON; take the first file_path or notebook_path value without needing jq.
  path=$(printf '%s' "$input" | sed -n 's/.*"\(file_path\|notebook_path\)"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\2/p' | head -n 1)
  [ -n "$path" ] || exit 0

  # Start from the nearest existing directory, since Write may create new directories.
  dir=$(dirname -- "$path")
  while [ ! -d "$dir" ] && [ "$dir" != "/" ] && [ "$dir" != "." ]; do dir=$(dirname -- "$dir"); done
  top=$(git -C "$dir" rev-parse --show-toplevel 2>/dev/null) || exit 0

  # Only guard repositories that carry this hook.
  [ -f "$top/.claude/hooks/block-protected-paths.sh" ] || exit 0

  case "$path" in
    "$top"/target/* | "$top"/third-party/* | "$top"/Cargo.lock)
      printf 'Blocked: %s is generated or vendored. Let cargo update Cargo.lock and target/; change third-party/ by re-vendoring. Set JIDOKA_ALLOW_PROTECTED_EDITS=1 if this edit is intended.\n' "$path" >&2
      exit 2
      ;;
  esac
}

main "$@"
