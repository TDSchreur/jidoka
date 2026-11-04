#!/bin/sh
# Install the latest public TDS Factory release on Linux x86-64.
set -eu

main() {
  [ "$(uname -s)" = Linux ] && [ "$(uname -m)" = x86_64 ] || { echo 'Only Linux x86-64 is supported by this installer.' >&2; exit 1; }
  command -v curl >/dev/null || { echo 'curl is required.' >&2; exit 1; }
  command -v sha256sum >/dev/null || { echo 'sha256sum is required.' >&2; exit 1; }
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
  "$HOME/.local/bin/factory" install --agent "${FACTORY_AGENT:-codex}"
  echo 'Installed factory. Ensure ~/.local/bin is on PATH, then run factory dashboard.'
}
main "$@"
