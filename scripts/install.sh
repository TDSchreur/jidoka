#!/bin/sh
# Install the latest public TDS Factory release on Linux x86-64.
# Choose the agents to install skills for with FACTORY_AGENT (for example codex,claude)
# or answer the prompt.
set -eu

# Turns a choice such as "1,3", "claude copilot" or "all" into "codex,copilot".
parse_choice() (
  set -f
  chosen=''
  for token in $(printf '%s' "$1" | tr ',' ' '); do
    case "$token" in
      1 | codex) names='codex' ;;
      2 | claude) names='claude' ;;
      3 | copilot) names='copilot' ;;
      all) names='codex claude copilot' ;;
      *) return 1 ;;
    esac
    for name in $names; do
      case " $chosen " in
        *" $name "*) ;;
        *) chosen="$chosen $name" ;;
      esac
    done
  done
  [ -n "$chosen" ] || return 1
  printf '%s\n' "$chosen" | sed 's/^ //; s/ /,/g'
)

# Prints the chosen agents, asking on the terminal unless FACTORY_AGENT is set.
choose_agents() {
  if [ -n "${FACTORY_AGENT:-}" ]; then
    parse_choice "$FACTORY_AGENT" || { echo "FACTORY_AGENT must list codex, claude, or copilot, separated by commas." >&2; return 1; }
    return 0
  fi
  # The script itself may be piped into sh, so ask on the terminal instead of stdin.
  if ! (: </dev/tty) 2>/dev/null; then
    echo 'No terminal to ask which agents to install for. Set FACTORY_AGENT, for example FACTORY_AGENT=codex,claude.' >&2
    return 1
  fi
  while :; do
    printf '%s' 'Install the TDS Factory skills for which agents?
  1) codex
  2) claude
  3) copilot
Enter one or more numbers or names separated by commas, or "all": ' >/dev/tty
    read -r answer </dev/tty || { echo 'No answer given.' >&2; return 1; }
    parse_choice "$answer" && return 0
    echo 'Invalid choice. Try again.' >/dev/tty
  done
}

main() {
  [ "$(uname -s)" = Linux ] && [ "$(uname -m)" = x86_64 ] || { echo 'Only Linux x86-64 is supported by this installer.' >&2; exit 1; }
  command -v curl >/dev/null || { echo 'curl is required.' >&2; exit 1; }
  command -v sha256sum >/dev/null || { echo 'sha256sum is required.' >&2; exit 1; }
  agents=$(choose_agents) || exit 1
  base='https://github.com/TDSchreur/tds-factory2/releases/latest/download'
  temp=$(mktemp -d)
  trap 'rm -rf "$temp"' EXIT HUP INT TERM
  curl -fsSL "$base/factory-linux-x86_64" -o "$temp/factory"
  curl -fsSL "$base/factory-linux-x86_64.sha256" -o "$temp/checksum"
  expected=$(awk '{print $1}' "$temp/checksum")
  actual=$(sha256sum "$temp/factory" | awk '{print $1}')
  [ -n "$expected" ] && [ "$actual" = "$expected" ] || { echo 'Download checksum mismatch.' >&2; exit 1; }
  mkdir -p "$HOME/.local/bin"
  chmod 755 "$temp/factory"
  # Copy to a temporary file in the destination before replacing the executable.
  cp "$temp/factory" "$HOME/.local/bin/factory.new"
  mv -f "$HOME/.local/bin/factory.new" "$HOME/.local/bin/factory"
  "$HOME/.local/bin/factory" install --agent "$agents"
  echo 'Installed factory. Ensure ~/.local/bin is on PATH, then run factory dashboard.'
}
main "$@"
