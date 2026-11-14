#!/bin/sh
# Switch between the official Jidoka release and the build from this checkout.
#
#   scripts/switch-jidoka.sh official   remove the local build, install the latest release
#   scripts/switch-jidoka.sh local      remove the official release, install this checkout
#   scripts/switch-jidoka.sh status     show which installs exist
#
# The other install is always removed first, so only one `jidoka` is on PATH. Both installs refresh
# the skills with `jidoka install`; set JIDOKA_AGENT (names, numbers or `all`) and
# JIDOKA_CLAUDE_CONFIG_DIRS (separated by colons) to skip its questions. A running dashboard is
# stopped, not restarted: start it again with `jidoka dashboard`.
set -eu

official_bin() { printf '%s/.local/bin/jidoka\n' "$HOME"; }
local_bin() { printf '%s/bin/jidoka\n' "${CARGO_HOME:-$HOME/.cargo}"; }

usage() {
  printf 'Usage: %s official|local|status\n' "$0" >&2
  exit 2
}

show() {
  if [ -x "$2" ]; then
    printf '%-9s %s (%s)\n' "$1" "$2" "$("$2" --version 2>&1 | head -n 1)"
  else
    printf '%-9s not installed\n' "$1"
  fi
}

status() {
  show official "$(official_bin)"
  show local "$(local_bin)"
}

# Stops any running `jidoka dashboard`; run data lives on disk, so nothing is lost.
stop_dashboard() {
  command -v pkill >/dev/null || return 0
  pattern='(^|/)jidoka dashboard$'
  pkill -f "$pattern" || return 0
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    pgrep -f "$pattern" >/dev/null || return 0
    sleep 0.5
  done
  echo 'The old dashboard did not stop.' >&2
  return 1
}

remove_official() {
  if [ -e "$(official_bin)" ]; then
    rm -f -- "$(official_bin)"
    echo "Removed the official install: $(official_bin)"
  fi
}

remove_local() {
  if cargo install --list 2>/dev/null | grep -q '^jidoka v'; then
    cargo uninstall jidoka
  else
    rm -f -- "$(local_bin)"
  fi
}

# Runs `jidoka install`, which asks for the agents itself unless JIDOKA_AGENT is set. The question is
# read from /dev/tty when stdin is not a terminal.
install_skills() {
  if [ -t 0 ] || ! (: </dev/tty) 2>/dev/null; then
    "$1" install
  else
    "$1" install </dev/tty
  fi
}

switch_official() {
  root=$1
  stop_dashboard
  remove_local
  sh "$root/scripts/install.sh"
  hash -r 2>/dev/null || true
}

switch_local() {
  root=$1
  command -v cargo >/dev/null || { echo 'cargo is required.' >&2; exit 1; }
  stop_dashboard
  remove_official
  cargo install --path "$root" --locked --force
  install_skills "$(local_bin)"
  case ":$PATH:" in
    *":${CARGO_HOME:-$HOME/.cargo}/bin:"*) ;;
    *) echo "Add ${CARGO_HOME:-$HOME/.cargo}/bin to PATH so the jidoka command resolves." >&2 ;;
  esac
}

main() {
  [ "$#" -eq 1 ] || usage
  root=$(cd "$(dirname "$0")/.." && pwd)
  case "$1" in
    official) switch_official "$root" ;;
    local) switch_local "$root" ;;
    status) status; exit 0 ;;
    *) usage ;;
  esac
  status
}
main "$@"
