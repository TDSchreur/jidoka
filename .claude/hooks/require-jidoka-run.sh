#!/bin/sh
# PreToolUse hook: AGENTS.md says every change goes through a jidoka run, and a run works in its own git worktree.
# Deny edits to files in this repository's main checkout; allow linked worktrees and files outside the repository.
# Set JIDOKA_ALLOW_MAIN_EDITS=1 when the user explicitly says to work without jidoka.
set -eu

main() {
  [ "${JIDOKA_ALLOW_MAIN_EDITS:-}" = 1 ] && exit 0
  input=$(cat)
  # The tool input is JSON; take the first file_path or notebook_path value without needing jq.
  path=$(printf '%s' "$input" | sed -n 's/.*"\(file_path\|notebook_path\)"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\2/p' | head -n 1)
  [ -n "$path" ] || exit 0

  # Start from the nearest existing directory, since Write may create new directories.
  dir=$(dirname -- "$path")
  while [ ! -d "$dir" ] && [ "$dir" != "/" ] && [ "$dir" != "." ]; do dir=$(dirname -- "$dir"); done

  git_dir=$(git -C "$dir" rev-parse --path-format=absolute --git-dir 2>/dev/null) || exit 0
  common_dir=$(git -C "$dir" rev-parse --path-format=absolute --git-common-dir 2>/dev/null) || exit 0
  top=$(git -C "$dir" rev-parse --show-toplevel 2>/dev/null) || exit 0

  # Only guard repositories that carry this hook.
  [ -f "$top/.claude/hooks/require-jidoka-run.sh" ] || exit 0
  [ "$git_dir" != "$common_dir" ] && exit 0

  printf 'Blocked: %s is in the main checkout. Start or continue a jidoka run (the jidoka skill) and edit in its worktree.\n' "$path" >&2
  exit 2
}

main "$@"
