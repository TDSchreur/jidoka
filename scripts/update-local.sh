#!/bin/sh
# Update this checkout to the latest develop, rebuild jidoka, reinstall the `jidoka`
# command and its skills, and restart the dashboard. Set JIDOKA_AGENT (names, numbers or `all`,
# for example `all` or `codex,claude`) to skip the question about which agents to install for;
# JIDOKA_CLAUDE_CONFIG_DIRS (separated by colons) selects the Claude configuration directories.
set -eu

# Stops any running `jidoka dashboard`; run data lives on disk, so nothing is lost.
stop_dashboard() {
  pattern='(^|/)jidoka dashboard$'
  pkill -f "$pattern" || return 0
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    pgrep -f "$pattern" >/dev/null || return 0
    sleep 0.5
  done
  echo 'The old dashboard did not stop.' >&2
  return 1
}

start_dashboard() {
  log_dir="${XDG_STATE_HOME:-$HOME/.local/state}/jidoka"
  mkdir -p "$log_dir"
  nohup "$1" dashboard >"$log_dir/dashboard.log" 2>&1 &
  pid=$!
  sleep 1
  kill -0 "$pid" 2>/dev/null || { echo "Dashboard failed to start; see $log_dir/dashboard.log" >&2; return 1; }
  echo "Dashboard restarted (pid $pid, log $log_dir/dashboard.log)."
}

# Restores the changes stashed by main, also when a later step failed.
restore_changes() {
  trap - EXIT
  if git stash pop --index >/dev/null; then
    echo 'Restored local changes.'
  else
    echo 'Could not restore local changes cleanly; they remain in the stash (git stash list).' >&2
  fi
}

# Installs the skills with the built binary, which reads JIDOKA_AGENT and JIDOKA_CLAUDE_CONFIG_DIRS
# itself and asks on the terminal when JIDOKA_AGENT is unset. The script may be piped into sh, so
# the question is read from /dev/tty instead of stdin.
install_skills() {
  if (: </dev/tty) 2>/dev/null; then
    "$1" install </dev/tty
  else
    "$1" install
  fi
}

main() {
  command -v pkill >/dev/null || { echo 'pkill (procps) is required.' >&2; exit 1; }
  root=$(git rev-parse --show-toplevel)
  cd "$root"
  if [ -n "$(git status --porcelain)" ]; then
    git stash push --include-untracked --message 'update-local.sh autostash' >/dev/null
    echo 'Stashed local changes; they are restored when the script ends.'
    trap restore_changes EXIT
    trap 'exit 1' HUP INT TERM
  fi
  git fetch origin develop
  git checkout develop
  git pull --ff-only origin develop
  cargo build --release --locked
  cargo install --path . --locked --force
  command -v jidoka >/dev/null 2>&1 || echo "Add ${CARGO_HOME:-$HOME/.cargo}/bin to PATH so the Jidoka command resolves." >&2
  install_skills "$root/target/release/jidoka"
  "$root/target/release/jidoka" --version
  stop_dashboard
  start_dashboard "$root/target/release/jidoka"
}
main "$@"
