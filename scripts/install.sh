#!/bin/sh
# Install the latest public Jidoka release on Linux x86-64.
# `jidoka install` asks which agents to install skills for; JIDOKA_AGENT (for example codex,claude) and
# JIDOKA_CLAUDE_CONFIG_DIRS (colon-separated) skip the questions.
set -eu

# Runs `jidoka install`, which asks for the agents itself. JIDOKA_AGENT and JIDOKA_CLAUDE_CONFIG_DIRS reach it
# through the environment. When the script is piped into sh, stdin is not a terminal, so the shell redirects
# the binary's stdin from /dev/tty when a terminal can be opened; otherwise stdin stays as it is and the binary
# reports that it has no terminal to ask on.
install_skills() {
  if [ -t 0 ] || ! (: </dev/tty) 2>/dev/null; then
    "$1" install
  else
    "$1" install </dev/tty
  fi
}

main() {
  [ "$(uname -s)" = Linux ] && [ "$(uname -m)" = x86_64 ] || { echo 'Only Linux x86-64 is supported by this installer.' >&2; exit 1; }
  command -v curl >/dev/null || { echo 'curl is required.' >&2; exit 1; }
  command -v sha256sum >/dev/null || { echo 'sha256sum is required.' >&2; exit 1; }
  base='https://github.com/TDSchreur/jidoka/releases/latest/download'
  temp=$(mktemp -d)
  trap 'rm -rf "$temp"' EXIT HUP INT TERM
  curl -fsSL "$base/jidoka-linux-x86_64" -o "$temp/jidoka"
  curl -fsSL "$base/jidoka-linux-x86_64.sha256" -o "$temp/checksum"
  expected=$(awk '{print $1}' "$temp/checksum")
  actual=$(sha256sum "$temp/jidoka" | awk '{print $1}')
  [ -n "$expected" ] && [ "$actual" = "$expected" ] || { echo 'Download checksum mismatch.' >&2; exit 1; }
  mkdir -p "$HOME/.local/bin"
  chmod 755 "$temp/jidoka"
  # Copy to a temporary file in the destination before replacing the executable.
  cp "$temp/jidoka" "$HOME/.local/bin/jidoka.new"
  mv -f "$HOME/.local/bin/jidoka.new" "$HOME/.local/bin/jidoka"
  install_skills "$HOME/.local/bin/jidoka"
  echo 'Installed jidoka. Ensure ~/.local/bin is on PATH, then run jidoka dashboard.'
}
main "$@"
