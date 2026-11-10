#!/bin/sh
# Switch the local jidoka installation between the latest release and develop.
set -eu

# Installs the skills, passing each entry of JIDOKA_CLAUDE_CONFIG_DIRS (separated by colons) as
# --config-dir. The positional parameters serve as an array, so paths with spaces stay intact.
install_skills() {
  binary=$1
  agents=$2
  dirs=${JIDOKA_CLAUDE_CONFIG_DIRS:-}
  set -- install --agent "$agents"
  old_ifs=$IFS
  IFS=:
  set -f
  for dir in $dirs; do
    if [ -n "$dir" ]; then set -- "$@" --config-dir "$dir"; fi
  done
  set +f
  IFS=$old_ifs
  "$binary" "$@"
}

main() {
  case "${1:-}" in
    release | develop) mode=$1 ;;
    *) printf 'Usage: %s <release|develop>\n' "$0" >&2; exit 1 ;;
  esac
  [ "$#" -eq 1 ] || { printf 'Expected one argument.\n' >&2; exit 1; }
  for tool in git cargo curl sha256sum pkill mktemp; do
    command -v "$tool" >/dev/null 2>&1 || { printf '%s is required.\n' "$tool" >&2; exit 1; }
  done
  [ "$(uname -s)" = Linux ] && [ "$(uname -m)" = x86_64 ] || { printf 'Only Linux x86-64 is supported.\n' >&2; exit 1; }
  root=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
  temp=$(mktemp -d)
  trap 'rm -rf "$temp"' EXIT
  trap 'exit 1' HUP INT TERM

  if [ "$mode" = develop ]; then
    git -C "$root" fetch origin develop
    commit=$(git -C "$root" rev-parse FETCH_HEAD)
    git clone --quiet --shared --no-checkout "$root" "$temp/source"
    git -C "$temp/source" checkout --quiet "$commit"
    CARGO_TARGET_DIR="$temp/target" cargo build --manifest-path "$temp/source/Cargo.toml" --release --locked
    binary="$temp/target/release/jidoka"
  else
    base='https://github.com/TDSchreur/jidoka/releases/latest/download'
    curl -fsSL "$base/jidoka-linux-x86_64" -o "$temp/jidoka"
    curl -fsSL "$base/jidoka-linux-x86_64.sha256" -o "$temp/checksum"
    expected=$(awk '{print $1}' "$temp/checksum")
    actual=$(sha256sum "$temp/jidoka" | awk '{print $1}')
    [ -n "$expected" ] && [ "$actual" = "$expected" ] || { printf 'Download checksum mismatch.\n' >&2; exit 1; }
    binary="$temp/jidoka"
  fi

  agents=${JIDOKA_AGENT:-}
  if [ -z "$agents" ]; then
    printf 'Agents to install skills for (codex,claude,copilot): ' >/dev/tty
    read -r agents </dev/tty || { printf 'Set JIDOKA_AGENT to select agents.\n' >&2; exit 1; }
  fi
  case "$agents" in *[!a-z,]* | '' | ,* | *, | *,,*) printf 'Invalid agent list.\n' >&2; exit 1 ;; esac
  old_ifs=$IFS
  IFS=,
  for agent in $agents; do
    case "$agent" in codex | claude | copilot) ;; *) printf 'Unknown agent: %s\n' "$agent" >&2; exit 1 ;; esac
  done
  IFS=$old_ifs

  pkill -f '(^|/)jidoka dashboard($| )' || :
  cargo_bin="${CARGO_HOME:-$HOME/.cargo}/bin/jidoka"
  local_bin="$HOME/.local/bin/jidoka"
  if cargo install --list | grep -q '^jidoka v'; then
    cargo uninstall jidoka
  fi
  rm -f "$cargo_bin" "$local_bin"
  mkdir -p "$HOME/.local/bin"
  cp "$binary" "$local_bin.new"
  chmod 755 "$local_bin.new"
  mv -f "$local_bin.new" "$local_bin"
  install_skills "$local_bin" "$agents"
  "$local_bin" --version
  printf 'Installed %s. Ensure ~/.local/bin is on PATH and restart the dashboard if needed.\n' "$mode"
}
main "$@"
